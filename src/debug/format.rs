//! Response-formatting helpers for the debug server.

use rmcp::model::{CallToolResult, Content};

use super::child::ChildHandle;
use super::config::ConfigState;

// ── JSON-RPC response unwrapping ──────────────────────────────────────────────

/// Unwrap a raw JSON-RPC response string into a `CallToolResult`.
pub fn unwrap_jsonrpc_response(method: &str, raw: &str) -> CallToolResult {
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

    // tools/call results are CallToolResult-shaped — pass through transparently
    if method == "tools/call" {
        if let Ok(ctr) = serde_json::from_value::<CallToolResult>(result.clone()) {
            return ctr;
        }
    }

    CallToolResult::success(vec![Content::text(format_mcp_result(method, result))])
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
            let short = desc.lines().next().unwrap_or("");
            Some(format!("- {name}: {short}"))
        })
        .collect::<Vec<_>>()
        .join("\n")
}

// ── tool body formatters ──────────────────────────────────────────────────────

/// Format the text for the `status` tool response.
pub async fn format_status(
    cfg: &ConfigState,
    child_guard: &Option<ChildHandle>,
    is_proxying: bool,
) -> String {
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
    lines.join("\n")
}

/// Format the text for the `show_config` tool response.
pub fn format_show_config(cli_lsp_specs: &[String], cfg: &ConfigState) -> String {
    let mut lines = vec![format!("Config file: {}", cfg.path.display())];

    if let Some(err) = &cfg.load_error {
        lines.push(format!("⚠ Load error: {err}"));
    }
    lines.push(String::new());
    lines.push("Command-line LSPs:".to_string());
    if cli_lsp_specs.is_empty() {
        lines.push("  (none)".to_string());
    } else {
        for s in cli_lsp_specs {
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
    lines.join("\n")
}
