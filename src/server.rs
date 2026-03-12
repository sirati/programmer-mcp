use std::sync::Arc;

use rmcp::{
    handler::server::{router::tool::ToolRouter, wrapper::Parameters},
    model::{CallToolResult, Content, Implementation, ServerCapabilities, ServerInfo},
    schemars, tool, tool_handler, tool_router, ErrorData as McpError, ServerHandler,
};

use crate::background::BackgroundManager;
use crate::ipc::HumanMessageBus;
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
    message_bus: Arc<HumanMessageBus>,
    background: Arc<BackgroundManager>,
    tool_router: ToolRouter<Self>,
}

#[tool_router]
impl ProgrammerServer {
    pub fn new(
        manager: Arc<LspManager>,
        message_bus: Arc<HumanMessageBus>,
        background: Arc<BackgroundManager>,
    ) -> Self {
        Self {
            manager,
            message_bus,
            background,
            tool_router: Self::tool_router(),
        }
    }

    #[tool(
        description = "Execute one or more language server operations in parallel.\n\
        CRITICAL: ALWAYS batch ALL related operations into a single call. \
        NEVER make separate calls when multiple operations can be combined. \
        Every call should contain as many operations as possible.\n\n\
        Supported operations:\n\
        LSP / code navigation:\n\
        - definition: get symbol source (symbolNames: string or array)\n\
        - references: find all usages (symbolNames: string or array)\n\
        - list_symbols: tree of symbols in a file (filePath, maxDepth)\n\
        - docstring: get doc comments (symbolNames: string or array)\n\
        - body: get symbol source body (symbolNames: string or array)\n\
        - impls: find all impl blocks for a type (symbolNames: string or array)\n\
        - diagnostics: get file errors/warnings (filePath)\n\
        - hover: get type/docs at position (filePath, line, column)\n\
        - rename_symbol: rename across project (filePath, line, column, newName)\n\
        - raw_lsp_request: raw LSP query (method, params, language)\n\n\
        Background processes:\n\
        - start_process: start a named background process (name, command, args, group)\n\
        - stop_process: stop a background process by name\n\
        - search_process_output: grep background process output (name/group, pattern)\n\
        - define_trigger: define a trigger on process output (name, pattern, linesBefore, linesAfter, timeoutMs, group)\n\
        - await_trigger: wait for a trigger to fire (name)\n\n\
        Task management (saved to .programmer-mcp/tasks/):\n\
        - set_task: create/replace a task (name, description)\n\
        - update_task: update description/appendDescription/completed flag (name, ...)\n\
        - add_subtask: add a subtask to a task (taskName, subtaskName, description)\n\
        - complete_task: mark a task done (name)\n\
        - complete_subtask: mark a subtask done (taskName, subtaskName)\n\
        - list_tasks: list pending tasks; pass includeCompleted=true for all\n\
        - list_subtasks: list pending subtasks of a task (taskName, includeCompleted)\n\n\
        Misc:\n\
        - request_human_message: block until human sends a message\n\n\
        Each LSP operation can optionally specify 'language' to target a specific LSP."
    )]
    async fn execute(
        &self,
        Parameters(request): Parameters<ExecuteRequest>,
    ) -> Result<CallToolResult, McpError> {
        let results = tools::execute_batch(
            &self.manager,
            &self.message_bus,
            &self.background,
            request.operations,
        )
        .await;

        let mut output = format_results(&results);
        let any_error = results.iter().any(|r| !r.success);

        // Append any pending trigger results
        let pending_triggers = self.background.triggers.lock().await.take_pending();
        for tr in &pending_triggers {
            output.push_str(&format!("\n\n{tr}"));
        }

        // Append any pending human messages
        let pending = self.message_bus.take_pending().await;
        if !pending.is_empty() {
            output.push_str("\n\n--- Human Message ---\n");
            output.push_str(&pending.join("\n"));
        }

        if any_error {
            Ok(CallToolResult::error(vec![Content::text(output)]))
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
                "Multi-language LSP + task management server.\n\
                 RULE: ALWAYS pass multiple operations in a single 'execute' call. \
                 Never issue separate calls when the operations are independent — \
                 they run in parallel and the combined result is returned at once.\n\
                 Use 'definition'/'body' to read code, 'references' to find usages, \
                 'list_symbols' to explore a file, 'diagnostics' for errors, \
                 'start_process'/'define_trigger'/'await_trigger' for build/test workflows, \
                 and 'set_task'/'list_tasks' to track work items.",
            )
    }
}

fn format_results(results: &[OperationResult]) -> String {
    if results.len() == 1 {
        return results[0].output.clone();
    }

    let mut sections = Vec::new();
    let mut empty_count = 0;
    let mut error_count = 0;

    for r in results {
        if !r.success {
            error_count += 1;
            sections.push(format!("{} failed: {}", r.operation, r.output));
        } else if r.output.is_empty() || r.output.ends_with("not found") {
            empty_count += 1;
        } else {
            sections.push(r.output.clone());
        }
    }

    if empty_count > 0 && !sections.is_empty() {
        sections.push("Some requests found nothing".to_string());
    }

    if sections.is_empty() {
        if error_count > 0 {
            "All operations failed".to_string()
        } else {
            "No results found".to_string()
        }
    } else {
        sections.join("\n\n---\n\n")
    }
}
