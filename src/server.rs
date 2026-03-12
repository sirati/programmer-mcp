use std::path::PathBuf;
use std::sync::Arc;

use rmcp::{
    handler::server::{router::tool::ToolRouter, wrapper::Parameters},
    model::{CallToolResult, Content, Implementation, ServerCapabilities, ServerInfo},
    schemars, tool, tool_handler, tool_router, ErrorData as McpError, ServerHandler,
};

use crate::background::BackgroundManager;
use crate::config::LengthLimits;
use crate::ipc::HumanMessageBus;
use crate::lsp::manager::LspManager;
use crate::tools::diagnostics_cache::DiagnosticsCache;
use crate::tools::edit::{PendingEdits, UndoStore};
use crate::tools::{self, OperationResult};

/// The batch request: a DSL script of commands to execute.
#[derive(Debug, serde::Deserialize, rmcp::schemars::JsonSchema)]
pub struct ExecuteRequest {
    /// Multi-line DSL script. Each line is one command; `#` starts a comment.
    /// Use `|` to separate multiple commands on a single line.
    ///
    /// Navigation:
    ///   cd <dir>                  — set directory context (no extension = folder)
    ///   cd <file.ext>             — set file context (extension required)
    ///
    /// File-based operations (use [file list] or current cd file):
    ///   list_symbols [f1 f2]      — symbol tree of files; on a directory: lists source files
    ///   diagnostics [f1 f2]       — errors/warnings for files
    ///   hover <file> <line> <col> — hover info at position (file optional if cd'd)
    ///   rename_symbol <file> <line> <col> <new_name>
    ///   code_action <file> <line> <col> [end_line end_col] [kind1 kind2]
    ///
    /// Symbol-based operations:
    ///   body        [sym1 sym2]   — source body of symbols
    ///   definition  [sym1 sym2]   — definition location
    ///   references  [sym1 sym2]   — all usages
    ///   docstring   [sym1 sym2]   — doc comments
    ///   impls       [sym1 sym2]   — impl blocks (Rust)
    ///   callers     [sym1 sym2]   — find callers (incoming calls)
    ///   callees     [sym1 sym2]   — find callees (outgoing calls)
    ///
    /// Item lists: [a, b, tools/{mod.rs x.rs}] — commas/spaces as separators,
    /// brace expansion: tools/{mod.rs x.rs} → tools/mod.rs, tools/x.rs
    ///
    /// Task management:
    ///   set_task <name> <description>
    ///   update_task <name> <new_description>
    ///   update_task <name> append=<text>
    ///   complete_task <name>
    ///   list_tasks [completed]
    ///   add_subtask <task> <sub> <description>
    ///   complete_subtask <task> <sub>
    ///   list_subtasks <task> [completed]
    ///
    /// Background processes & triggers:
    ///   start_process <name> <command> [args...] [group=<g>]
    ///   stop_process <name>
    ///   search_output <name> <pattern>
    ///   define_trigger <name> <pattern> [before=N] [after=N] [timeout=N] [group=g]
    ///   await_trigger <name>
    ///
    /// Refactoring:
    ///   code_actions <file> <line> <col>   — list available code actions
    ///   apply_action <file> <line> <col> <index> — apply a code action by index
    ///   format [files]                     — format files
    ///
    /// Read & Search:
    ///   read <file> [start end]            — read file contents (line range optional)
    ///   grep <pattern>                     — search workspace files for text (scoped by cd)
    ///
    /// Misc:
    ///   workspace_info                     — show subprojects and standalone files
    ///   request_human_message
    pub commands: String,
}

#[derive(Clone)]
pub struct ProgrammerServer {
    manager: Arc<LspManager>,
    message_bus: Arc<HumanMessageBus>,
    background: Arc<BackgroundManager>,
    workspace_root: PathBuf,
    diag_cache: Arc<DiagnosticsCache>,
    pending_edits: PendingEdits,
    undo_store: UndoStore,
    allow_file_edit: bool,
    length_limits: LengthLimits,
    tool_router: ToolRouter<Self>,
}

#[tool_router]
impl ProgrammerServer {
    pub fn new(
        manager: Arc<LspManager>,
        message_bus: Arc<HumanMessageBus>,
        background: Arc<BackgroundManager>,
        workspace_root: PathBuf,
        diag_cache: Arc<DiagnosticsCache>,
        allow_file_edit: bool,
        length_limits: LengthLimits,
    ) -> Self {
        Self {
            manager,
            message_bus,
            background,
            workspace_root,
            diag_cache,
            pending_edits: crate::tools::edit::new_pending_edits(),
            undo_store: crate::tools::edit::new_undo_store(),
            allow_file_edit,
            length_limits,
            tool_router: Self::tool_router(),
        }
    }

    #[tool(
        description = "Execute language server operations using a DSL script.\n\
        CRITICAL: ALWAYS batch ALL related commands into a single call — pack as many\n\
        commands as possible per invocation.\n\n\
        Each line is one command; `#` starts a comment.\n\n\
        NAVIGATION:\n\
          cd src/debug            # directory context (no extension)\n\
          cd src/debug/server.rs  # file context (extension required)\n\n\
        FILE-BASED OPS (use [list] or current cd file):\n\
          list_symbols [server.rs child.rs]\n\
          list_symbols                     # uses current cd file\n\
          diagnostics [server.rs]\n\
          hover src/main.rs 42 10\n\
          hover 42 10                      # uses current cd file\n\
          rename_symbol src/main.rs 42 10 new_name\n\
          code_action src/main.rs 42 10    # code actions at position\n\
          code_action 42 10 50 15 refactor # range + kind filter\n\n\
        SYMBOL-BASED OPS:\n\
          body        [relay_command show_help]\n\
          definition  [MyStruct MyStruct.method]\n\
          references  [my_fn]\n\
          docstring   [MyTrait]\n\
          impls       [MyType]\n\n\
        LIST SYNTAX: [a, b, tools/{mod.rs x.rs}]\n\
          • separators: space, comma, or both\n\
          • brace expansion: tools/{mod.rs x.rs} → tools/mod.rs tools/x.rs\n\
          • find_{a b} → find_a find_b\n\n\
        TASKS:\n\
          set_task task-name Description text\n\
          update_task task-name New description\n\
          update_task task-name append=More text\n\
          complete_task task-name\n\
          list_tasks [completed]\n\
          add_subtask task-name sub-name Description\n\
          complete_subtask task-name sub-name\n\
          list_subtasks task-name [completed]\n\n\
        BACKGROUND & TRIGGERS:\n\
          start_process myproc cargo test [group=build]\n\
          stop_process myproc\n\
          search_output myproc error\n\
          define_trigger t error [before=3] [after=5] [timeout=30000] [group=build]\n\
          await_trigger t\n\n\
        EDITING:\n\
          edit body path/file.rs symbol_name <new content>\n\
          edit signature path/file.rs symbol_name <new sig>\n\
          edit docs path/file.rs symbol_name <new docs>\n\
          edit body,docs path/file.rs sym <content>  # multiple types\n\
          apply_edit <id>                             # confirm with stored args\n\
          apply_edit <id> [signature body]             # override edit types\n\
          apply_edit <id> path/file.rs symbol_name     # correct location\n\
          undo <id>                                    # revert an applied edit\n\
          edit_range path sym <<<before>>> new <<<after>>>  # targeted range edit\n\
          edit_range path sym new <<<after>>>               # from body start to anchor\n\
          edit_range path sym <<<before>>> new              # from anchor to body end\n\n\
        REFACTORING:\n\
          code_actions src/main.rs 42 10    # list available actions\n\
          code_actions 42 10                # uses current cd file\n\
          apply_action src/main.rs 42 10 0  # apply action by index\n\
          format src/main.rs                # format a file\n\
          format                            # format current cd file\n\n\
        MISC:\n\
          workspace_info                    # show subprojects & standalone files\n\
          request_human_message"
    )]
    async fn execute(
        &self,
        Parameters(request): Parameters<ExecuteRequest>,
    ) -> Result<CallToolResult, McpError> {
        let dsl_opts = tools::dsl::DslOptions {
            allow_file_edit: self.allow_file_edit,
        };
        let parsed = tools::dsl::parse_dsl_with_options(&request.commands, &dsl_opts);
        let results = tools::execute_batch(
            &self.manager,
            &self.message_bus,
            &self.background,
            &self.workspace_root,
            parsed.operations,
            &self.pending_edits,
            &self.undo_store,
            self.length_limits,
        )
        .await;

        let mut output = String::new();

        // Prepend DSL warnings
        if !parsed.warnings.is_empty() {
            for w in &parsed.warnings {
                output.push_str("⚠ ");
                output.push_str(w);
                output.push('\n');
            }
            output.push('\n');
        }

        output.push_str(&format_results(&results));
        let any_error = results.iter().any(|r| !r.success);

        // Nudge callers to batch commands when only one was sent
        if results.len() == 1 {
            output.push_str("\nPlease always batch multiple commands together.\n");
        }

        // Append any pending auto-diagnostics
        if let Some(diag_report) = self.diag_cache.take_pending().await {
            output.push_str("\n\n");
            output.push_str(&diag_report);
        }

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
                 RULE: ALWAYS pack as many commands as possible into a single 'execute' call.\n\
                 Never issue separate calls when operations are independent — they run in parallel.\n\
                 Use 'cd' + 'list_symbols'/'body'/'definition' to navigate code,\n\
                 'references' to find usages, 'diagnostics' for errors,\n\
                 'start_process'/'define_trigger'/'await_trigger' for build/test workflows,\n\
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
