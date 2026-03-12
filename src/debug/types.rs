//! Request parameter types for DebugServer tools.

/// Parameters for the `grab_log` tool.
#[derive(Debug, serde::Deserialize, rmcp::schemars::JsonSchema)]
pub struct GrabLogRequest {
    /// Optional substring to filter log lines
    pub query: Option<String>,
    /// Maximum number of most-recent lines to return (default 100)
    pub limit: Option<usize>,
}

/// Parameters for the `configure` tool.
#[derive(Debug, serde::Deserialize, rmcp::schemars::JsonSchema)]
pub struct ConfigureRequest {
    /// Add an LSP spec ("language:command [args]"), e.g. "rust:rust-analyzer --stdio"
    pub add_lsp: Option<String>,
    /// Remove all LSP specs for this language name
    pub remove_lsp: Option<String>,
}

/// Parameters for the `execute` relay tool.
#[derive(Debug, serde::Deserialize, rmcp::schemars::JsonSchema)]
pub struct ExecuteRequest {
    /// DSL command script — forwarded verbatim to the child's `execute` tool.
    /// See the child's `execute` tool description (via `show_help`) for full syntax.
    pub commands: String,
}
