use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;

use rmcp::{
    handler::server::{router::tool::ToolRouter, wrapper::Parameters},
    model::{
        CallToolRequestParams, CallToolResult, Content, Implementation, ListToolsResult,
        PaginatedRequestParams, ServerCapabilities, ServerInfo,
    },
    schemars, tool, tool_router, ErrorData as McpError, RoleServer, ServerHandler,
};
use tokio::sync::Mutex;

use super::build::run_cargo_build;
use super::child::ChildHandle;
use super::config::ConfigState;
use super::proxy;
use super::relay::build_jsonrpc_request;
use super::update::run_update_debug_bin;

// ── request types ─────────────────────────────────────────────────────────────

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct GrabLogRequest {
    /// Optional substring to filter log lines
    pub query: Option<String>,
    /// Maximum number of most-recent lines to return (default 100)
    pub limit: Option<usize>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct RelayCommandRequest {
    /// MCP method to call (e.g. "tools/list", "tools/call")
    pub method: String,
    /// Method parameters as a JSON object
    pub params: serde_json::Value,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct ConfigureRequest {
    /// Add an LSP spec ("language:command [args]"), e.g. "rust:rust-analyzer --stdio"
    pub add_lsp: Option<String>,
    /// Remove all LSP specs for this language name
    pub remove_lsp: Option<String>,
}

// ── server struct ─────────────────────────────────────────────────────────────

#[derive(Clone)]
pub struct DebugServer {
    project_root: PathBuf,
    cli_lsp_specs: Vec<String>,
    /// Original argv (skip(1)) — used to spawn debug children with identical flags.
    original_args: Vec<String>,
    config_state: Arc<Mutex<ConfigState>>,
    /// The normal (non-debug) child process managed by this server.
    child: Arc<Mutex<Option<ChildHandle>>>,
    /// The new debug child process (populated after update_debug_bin succeeds).
    debug_child: Arc<Mutex<Option<ChildHandle>>>,
    proxy_mode: Arc<AtomicBool>,
    next_id: Arc<AtomicU64>,
    tool_router: ToolRouter<Self>,
}

// ── tools ─────────────────────────────────────────────────────────────────────

#[tool_router]
impl DebugServer {
    pub fn new(
        project_root: PathBuf,
        cli_lsp_specs: Vec<String>,
        original_args: Vec<String>,
    ) -> Self {
        let config_state = ConfigState::load(&project_root);
        Self {
            project_root: project_root.clone(),
            cli_lsp_specs,
            original_args,
            config_state: Arc::new(Mutex::new(config_state)),
            child: Arc::new(Mutex::new(None)),
            debug_child: Arc::new(Mutex::new(None)),
            proxy_mode: Arc::new(AtomicBool::new(false)),
            next_id: Arc::new(AtomicU64::new(1)),
            tool_router: Self::tool_router(),
        }
    }

    #[tool(
        description = "Build the project with `cargo build`. On failure returns filtered \
        compiler errors. On success copies the new binary to a temp location, starts it, \
        waits for it to be ready, stops the old instance, and reports the result."
    )]
    async fn rebuild(&self) -> Result<CallToolResult, McpError> {
        let args = self.current_child_args().await;
        if args.iter().filter(|a| a.as_str() == "--lsp").count() == 0 {
            return Ok(CallToolResult::error(vec![Content::text(
                "No LSP servers configured. Use `configure` with `add_lsp` to add at least \
                 one LSP spec (e.g. \"rust:rust-analyzer\") before rebuilding.",
            )]));
        }

        let outcome = run_cargo_build(&self.project_root).await;
        if !outcome.success() {
            return Ok(CallToolResult::error(vec![Content::text(format!(
                "Build failed:\n{}",
                outcome.errors
            ))]));
        }
        let binary_src = outcome.binary_path.unwrap();
        match replace_child(&self.child, &binary_src, &args, &self.project_root).await {
            Ok(msg) => Ok(CallToolResult::success(vec![Content::text(msg)])),
            Err(e) => Ok(CallToolResult::error(vec![Content::text(format!(
                "Build succeeded but restart failed: {e}"
            ))])),
        }
    }

    #[tool(
        description = "Build with `cargo build`, test the new binary in --debug mode, \
        replace this debug server's own binary, restart as the new debug server, and \
        forward all subsequent traffic to it. When already in proxy mode, replaces the \
        debug child instead. Only `update_debug_bin` is handled locally; all other \
        tool calls are forwarded."
    )]
    async fn update_debug_bin(&self) -> Result<CallToolResult, McpError> {
        let outcome = run_update_debug_bin(
            &self.project_root,
            &self.original_args,
            &self.child,
            &self.debug_child,
            &self.proxy_mode,
        )
        .await;
        if outcome.success {
            Ok(CallToolResult::success(vec![Content::text(
                outcome.message,
            )]))
        } else {
            Ok(CallToolResult::error(vec![Content::text(outcome.message)]))
        }
    }

    #[tool(
        description = "Report whether the tested child process is currently running, \
        how long it has been up, and any configuration load errors."
    )]
    async fn status(&self) -> Result<CallToolResult, McpError> {
        let cfg = self.config_state.lock().await;
        let child_guard = self.child.lock().await;
        let is_proxying = self.proxy_mode.load(Ordering::Relaxed);

        let process_status = match child_guard.as_ref() {
            None => "No child process has been started yet. Use `rebuild` to build and launch it."
                .to_string(),
            Some(c) if c.is_alive().await => {
                let secs = c.started_at.elapsed().as_secs();
                format!(
                    "Child process is running (up {}m {}s).",
                    secs / 60,
                    secs % 60
                )
            }
            Some(_) => "Child process was started but has since exited.".to_string(),
        };

        let mut lines = vec![process_status];
        if is_proxying {
            lines.push("Proxy mode active — traffic is forwarded to the debug child.".to_string());
        }
        if let Some(err) = &cfg.load_error {
            lines.push(format!("⚠ Config load error: {err}"));
        }

        Ok(CallToolResult::success(vec![Content::text(
            lines.join("\n"),
        )]))
    }

    #[tool(description = "Add or remove LSP servers in the saved debug config \
        (debug-mcp.config.toml). Changes take effect on the next `rebuild`.")]
    async fn configure(
        &self,
        Parameters(req): Parameters<ConfigureRequest>,
    ) -> Result<CallToolResult, McpError> {
        let mut cfg = self.config_state.lock().await;
        let mut messages: Vec<String> = Vec::new();

        if let Some(spec) = req.add_lsp {
            match cfg.add_lsp(spec.clone()) {
                Ok(()) => messages.push(format!("Added LSP: {spec}")),
                Err(e) => messages.push(format!("Add failed: {e}")),
            }
        }
        if let Some(lang) = req.remove_lsp {
            match cfg.remove_lsp(&lang) {
                Ok(()) => messages.push(format!("Removed LSP for language: {lang}")),
                Err(e) => messages.push(format!("Remove failed: {e}")),
            }
        }
        if messages.is_empty() {
            messages
                .push("No action specified. Provide `add_lsp` and/or `remove_lsp`.".to_string());
        }

        Ok(CallToolResult::success(vec![Content::text(
            messages.join("\n"),
        )]))
    }

    #[tool(
        description = "Show the current configuration: command-line LSP specs, \
        saved LSP specs from debug-mcp.config.toml, and any config load error."
    )]
    async fn show_config(&self) -> Result<CallToolResult, McpError> {
        let cfg = self.config_state.lock().await;
        let mut lines = vec![format!("Config file: {}", cfg.path.display())];

        if let Some(err) = &cfg.load_error {
            lines.push(format!("⚠ Load error: {err}"));
        }
        lines.push(String::new());
        lines.push("Command-line LSPs:".to_string());
        if self.cli_lsp_specs.is_empty() {
            lines.push("  (none)".to_string());
        } else {
            for s in &self.cli_lsp_specs {
                lines.push(format!("  {s}"));
            }
        }
        lines.push(String::new());
        lines.push("Saved LSPs (debug-mcp.config.toml):".to_string());
        if cfg.config.lsp.is_empty() {
            lines.push("  (none)".to_string());
        } else {
            for s in &cfg.config.lsp {
                lines.push(format!("  {s}"));
            }
        }

        Ok(CallToolResult::success(vec![Content::text(
            lines.join("\n"),
        )]))
    }

    #[tool(
        description = "Retrieve recent stderr log lines from the currently running child \
        process. Optionally filter by a search query and cap the number of lines."
    )]
    async fn grab_log(
        &self,
        Parameters(req): Parameters<GrabLogRequest>,
    ) -> Result<CallToolResult, McpError> {
        let guard = self.child.lock().await;
        let Some(child) = guard.as_ref() else {
            return Ok(CallToolResult::error(vec![Content::text(
                "No child process is running. Use `rebuild` first.",
            )]));
        };
        let lines = child
            .search_logs(req.query.as_deref(), req.limit.unwrap_or(100))
            .await;
        let text = if lines.is_empty() {
            "(no matching log lines)".to_string()
        } else {
            lines.join("\n")
        };
        Ok(CallToolResult::success(vec![Content::text(text)]))
    }

    #[tool(
        description = "Relay an MCP JSON-RPC call to the running child process and return \
        its response. Provide the `method` (e.g. \"tools/call\") and optional `params`."
    )]
    async fn relay_command(
        &self,
        Parameters(req): Parameters<RelayCommandRequest>,
    ) -> Result<CallToolResult, McpError> {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let request_json = build_jsonrpc_request(id, &req.method, req.params);

        let guard = self.child.lock().await;
        let Some(child) = guard.as_ref() else {
            return Ok(CallToolResult::error(vec![Content::text(
                "No child process is running. Use `rebuild` first.",
            )]));
        };
        match child.relay(&request_json).await {
            Ok(resp) => Ok(unwrap_jsonrpc_response(&req.method, &resp)),
            Err(e) => Ok(CallToolResult::error(vec![Content::text(format!(
                "Relay failed: {e}"
            ))])),
        }
    }
}

// ── helpers ───────────────────────────────────────────────────────────────────

fn unwrap_jsonrpc_response(method: &str, raw: &str) -> CallToolResult {
    let Ok(val) = serde_json::from_str::<serde_json::Value>(raw) else {
        return CallToolResult::success(vec![Content::text(raw)]);
    };
    if let Some(err) = val.get("error") {
        let msg = err
            .get("message")
            .and_then(|m| m.as_str())
            .unwrap_or("Unknown error");
        let code = err.get("code").and_then(|c| c.as_i64());
        let text = match code {
            Some(c) => format!("Error {c}: {msg}"),
            None => msg.to_string(),
        };
        return CallToolResult::error(vec![Content::text(text)]);
    }
    let Some(result) = val.get("result") else {
        return CallToolResult::success(vec![Content::text(raw)]);
    };
    // tools/call results are already CallToolResult-shaped — pass through transparently
    if method == "tools/call" {
        if let Ok(ctr) = serde_json::from_value::<CallToolResult>(result.clone()) {
            return ctr;
        }
    }
    // For other methods, extract the meaningful content
    let text = format_mcp_result(method, result);
    CallToolResult::success(vec![Content::text(text)])
}

fn format_mcp_result(method: &str, result: &serde_json::Value) -> String {
    match method {
        "tools/list" => format_tools_list(result),
        _ => serde_json::to_string_pretty(result).unwrap_or_else(|_| result.to_string()),
    }
}

fn format_tools_list(result: &serde_json::Value) -> String {
    let Some(tools) = result.get("tools").and_then(|t| t.as_array()) else {
        return "(no tools)".to_string();
    };
    tools
        .iter()
        .filter_map(|t| {
            let name = t.get("name")?.as_str()?;
            let desc = t.get("description").and_then(|d| d.as_str()).unwrap_or("");
            // Truncate description to first line
            let short = desc.lines().next().unwrap_or("");
            Some(format!("- {name}: {short}"))
        })
        .collect::<Vec<_>>()
        .join("\n")
}

// ── ServerHandler: manual impl for proxy interception ─────────────────────────

impl ServerHandler for DebugServer {
    fn get_info(&self) -> ServerInfo {
        ServerInfo::new(ServerCapabilities::builder().enable_tools().build())
            .with_server_info(Implementation::new(
                "programmer-mcp-debug",
                env!("CARGO_PKG_VERSION"),
            ))
            .with_instructions(
                "Debug control server for programmer-mcp. Commands: `rebuild`, \
                 `update_debug_bin`, `status`, `configure`, `show_config`, `grab_log`, \
                 `relay_command`. After `update_debug_bin` succeeds, all tool calls except \
                 `update_debug_bin` are transparently forwarded to the new debug process.",
            )
    }

    async fn call_tool(
        &self,
        request: CallToolRequestParams,
        context: rmcp::service::RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        if self.proxy_mode.load(Ordering::Relaxed) && request.name != "update_debug_bin" {
            return proxy::proxy_call_tool(
                &self.debug_child,
                &self.next_id,
                &self.proxy_mode,
                request,
            )
            .await;
        }
        let tcc = rmcp::handler::server::tool::ToolCallContext::new(self, request, context);
        self.tool_router.call(tcc).await
    }

    async fn list_tools(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: rmcp::service::RequestContext<RoleServer>,
    ) -> Result<ListToolsResult, McpError> {
        if self.proxy_mode.load(Ordering::Relaxed) {
            return proxy::proxy_list_tools(&self.debug_child, &self.next_id, &self.proxy_mode)
                .await;
        }
        Ok(ListToolsResult {
            tools: self.tool_router.list_all(),
            meta: None,
            next_cursor: None,
        })
    }

    fn get_tool(&self, name: &str) -> Option<rmcp::model::Tool> {
        self.tool_router.get(name).cloned()
    }
}

impl DebugServer {
    async fn current_child_args(&self) -> Vec<String> {
        let cfg = self.config_state.lock().await;
        let mut args = vec![
            "--workspace".to_string(),
            self.project_root.display().to_string(),
        ];
        for spec in &self.cli_lsp_specs {
            args.push("--lsp".to_string());
            args.push(spec.clone());
        }
        for spec in &cfg.config.lsp {
            args.push("--lsp".to_string());
            args.push(spec.clone());
        }
        args
    }
}

async fn replace_child(
    child_mutex: &Mutex<Option<ChildHandle>>,
    binary_src: &std::path::Path,
    args: &[String],
    workspace: &std::path::Path,
) -> anyhow::Result<String> {
    let tmp_binary = copy_to_tmp(binary_src)?;
    let new_child = ChildHandle::spawn(&tmp_binary, args, workspace).await?;
    if let Err(exit_info) = new_child.wait_for_ready().await {
        let logs = new_child.search_logs(None, 30).await;
        new_child.kill().await;
        let log_snippet = if logs.is_empty() {
            "(no stderr output captured)".to_string()
        } else {
            logs.join("\n")
        };
        anyhow::bail!(
            "new child {} before becoming ready.\n\
             workspace: {}\n\
             args: {}\n\
             --- child stderr ---\n{log_snippet}",
            exit_info.describe(),
            workspace.display(),
            args.join(" "),
        );
    }
    let mut guard = child_mutex.lock().await;
    let had_previous = guard.is_some();
    if let Some(old) = guard.take() {
        old.kill().await;
    }
    *guard = Some(new_child);
    Ok(if had_previous {
        "Rebuilt and restarted.".to_string()
    } else {
        "Built and started.".to_string()
    })
}

fn copy_to_tmp(src: &std::path::Path) -> anyhow::Result<std::path::PathBuf> {
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();
    let tmp_dir = std::env::temp_dir().join(format!("programmer-mcp-debug-{ts}"));
    std::fs::create_dir_all(&tmp_dir)?;

    let name = src
        .file_name()
        .ok_or_else(|| anyhow::anyhow!("binary has no filename"))?;
    let dest = tmp_dir.join(name);
    std::fs::copy(src, &dest)?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = std::fs::metadata(&dest)?.permissions();
        perms.set_mode(perms.mode() | 0o111);
        std::fs::set_permissions(&dest, perms)?;
    }

    Ok(dest)
}
