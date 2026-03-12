//! Debug-mode MCP server.
//!
//! Manages a child `programmer-mcp` process (build, restart, log access)
//! and exposes `show_help` + `execute` relay tools that forward to the child.

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
use super::format::{format_show_config, format_status, unwrap_jsonrpc_response};
use super::proxy;
use super::relay::build_jsonrpc_request;
use super::spawn::replace_child;
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
pub struct ConfigureRequest {
    /// Add an LSP spec ("language:command [args]"), e.g. "rust:rust-analyzer --stdio"
    pub add_lsp: Option<String>,
    /// Remove all LSP specs for this language name
    pub remove_lsp: Option<String>,
}

/// Request for the `execute` relay tool.
#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct ExecuteRequest {
    /// DSL command script — forwarded verbatim to the child's `execute` tool.
    /// See the child's `execute` tool description (via `show_help`) for full syntax.
    pub commands: String,
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
    /// The debug child process (populated after update_debug_bin succeeds).
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
        Ok(match self.run_rebuild().await {
            Ok(msg) => CallToolResult::success(vec![Content::text(msg)]),
            Err(msg) => CallToolResult::error(vec![Content::text(msg)]),
        })
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
        Ok(if outcome.success {
            CallToolResult::success(vec![Content::text(outcome.message)])
        } else {
            CallToolResult::error(vec![Content::text(outcome.message)])
        })
    }

    #[tool(
        description = "Report whether the tested child process is currently running, \
        how long it has been up, and any configuration load errors."
    )]
    async fn status(&self) -> Result<CallToolResult, McpError> {
        let cfg_guard = self.config_state.lock().await;
        let child_guard = self.child.lock().await;
        let text = format_status(
            &*cfg_guard,
            &*child_guard,
            self.proxy_mode.load(Ordering::Relaxed),
        )
        .await;
        Ok(CallToolResult::success(vec![Content::text(text)]))
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
        let cfg_guard = self.config_state.lock().await;
        let text = format_show_config(&self.cli_lsp_specs, &*cfg_guard);
        Ok(CallToolResult::success(vec![Content::text(text)]))
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
        description = "List all tools available in the running child process. \
        Use this to discover the child's DSL syntax and available commands \
        before calling `execute`."
    )]
    async fn show_help(&self) -> Result<CallToolResult, McpError> {
        self.relay_to_child("tools/list", serde_json::json!({}))
            .await
    }

    #[tool(
        description = "Execute DSL commands on the child programmer-mcp process.\n\
        The `commands` string is forwarded verbatim — call `show_help` first to \
        see the full DSL syntax supported by the child.\n\n\
        Quick reference:\n\
          cd src/debug            # directory context\n\
          cd src/debug/server.rs  # file context (extension required)\n\
          list_symbols [f1 f2]    # or bare: list_symbols\n\
          body       [sym1 sym2]\n\
          definition [sym1 sym2]\n\
          references [sym1]\n\
          diagnostics [file.rs]\n\
          list_tasks / set_task name desc / complete_task name\n\
          start_process name cmd [args] [group=g]\n\
        Brace expansion: tools/{mod.rs x.rs} → tools/mod.rs tools/x.rs"
    )]
    async fn execute(
        &self,
        Parameters(req): Parameters<ExecuteRequest>,
    ) -> Result<CallToolResult, McpError> {
        self.relay_to_child(
            "tools/call",
            serde_json::json!({
                "name": "execute",
                "arguments": { "commands": req.commands }
            }),
        )
        .await
    }
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
                "Debug control server for programmer-mcp.\n\
                 Commands: `rebuild`, `update_debug_bin`, `status`, `configure`, \
                 `show_config`, `grab_log`, `show_help`, `execute`.\n\
                 Use `show_help` to inspect the child's tool list, then use `execute` \
                 with DSL commands to interact with the running child.",
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

// ── private helpers ───────────────────────────────────────────────────────────

impl DebugServer {
    /// Core build-and-restart logic shared by the `rebuild` tool and auto-startup.
    ///
    /// Returns `Ok(status_message)` on success, `Err(error_message)` on failure.
    /// Returns `Ok` with a skip message if no LSP servers are configured.
    pub async fn run_rebuild(&self) -> Result<String, String> {
        let args = self.current_child_args().await;
        if args.iter().filter(|a| a.as_str() == "--lsp").count() == 0 {
            return Err(
                "No LSP servers configured. Use `configure` with `add_lsp` to add at least \
                 one LSP spec (e.g. \"rust:rust-analyzer\") before rebuilding."
                    .to_string(),
            );
        }
        let outcome = run_cargo_build(&self.project_root).await;
        if !outcome.success() {
            return Err(format!("Build failed:\n{}", outcome.errors));
        }
        let binary_src = outcome.binary_path.unwrap();
        replace_child(&self.child, &binary_src, &args, &self.project_root)
            .await
            .map_err(|e| format!("Build succeeded but restart failed: {e}"))
    }

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

    /// Send a JSON-RPC request to the child and return a `CallToolResult`.
    async fn relay_to_child(
        &self,
        method: &str,
        params: serde_json::Value,
    ) -> Result<CallToolResult, McpError> {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let request_json = build_jsonrpc_request(id, method, params);

        let guard = self.child.lock().await;
        let Some(child) = guard.as_ref() else {
            return Ok(CallToolResult::error(vec![Content::text(
                "No child process is running. Use `rebuild` first.",
            )]));
        };
        match child.relay(&request_json).await {
            Ok(resp) => Ok(unwrap_jsonrpc_response(method, &resp)),
            Err(e) => Ok(CallToolResult::error(vec![Content::text(format!(
                "Relay failed: {e}"
            ))])),
        }
    }
}
