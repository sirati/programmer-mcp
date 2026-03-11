use std::sync::Arc;

use rmcp::{
    handler::server::{router::tool::ToolRouter, wrapper::Parameters},
    model::{CallToolResult, Content, Implementation, ServerCapabilities, ServerInfo},
    schemars, tool, tool_handler, tool_router, ErrorData as McpError, ServerHandler,
};

use crate::lsp::manager::LspManager;
use crate::tools::{self, Operation, OperationResult};

/// The batch request: an array of operations to execute in parallel.
#[derive(Debug, serde::Deserialize, rmcp::schemars::JsonSchema)]
pub struct ExecuteRequest {
    /// List of operations to execute concurrently
    pub operations: Vec<Operation>,
}

#[derive(Clone)]
pub struct ProgrammerServer {
    manager: Arc<LspManager>,
    tool_router: ToolRouter<Self>,
}

#[tool_router]
impl ProgrammerServer {
    pub fn new(manager: Arc<LspManager>) -> Self {
        Self {
            manager,
            tool_router: Self::tool_router(),
        }
    }

    #[tool(
        description = "Execute one or more language server operations in parallel. \
        Supported operations: definition (find symbol source), references (find all usages), \
        diagnostics (get file errors/warnings), hover (get type/docs at position), \
        rename_symbol (rename across project). Each operation can optionally specify a \
        'language' to target a specific LSP server."
    )]
    async fn execute(
        &self,
        Parameters(request): Parameters<ExecuteRequest>,
    ) -> Result<CallToolResult, McpError> {
        let results = tools::execute_batch(&self.manager, request.operations).await;

        let output = format_results(&results);
        let any_error = results.iter().any(|r| !r.success);

        if any_error {
            Ok(CallToolResult {
                content: vec![Content::text(output)],
                is_error: Some(true),
                ..Default::default()
            })
        } else {
            Ok(CallToolResult::success(vec![Content::text(output)]))
        }
    }
}

#[tool_handler]
impl ServerHandler for ProgrammerServer {
    fn get_info(&self) -> ServerInfo {
        ServerInfo::new(ServerCapabilities::builder().enable_tools().build())
            .with_server_info(Implementation::new(
                "programmer-mcp",
                env!("CARGO_PKG_VERSION"),
            ))
            .with_instructions(
                "Multi-language LSP server. Use the 'execute' tool with an array of operations \
                 to query definitions, references, diagnostics, hover info, or rename symbols. \
                 Operations run in parallel. Specify 'language' to target a specific LSP."
                    .into(),
            )
    }
}

fn format_results(results: &[OperationResult]) -> String {
    if results.len() == 1 {
        return results[0].output.clone();
    }

    results
        .iter()
        .enumerate()
        .map(|(i, r)| {
            let status = if r.success { "OK" } else { "ERROR" };
            format!(
                "=== Operation {} ({}) [{}] ===\n{}",
                i + 1,
                r.operation,
                status,
                r.output
            )
        })
        .collect::<Vec<_>>()
        .join("\n\n")
}
