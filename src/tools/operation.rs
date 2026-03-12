//! Operation types for batch execution.
//!
//! Defines the [`Operation`] enum (all supported DSL operations)
//! and the [`OperationResult`] type returned by each executed operation.

use serde::{Deserialize, Serialize};

use super::serde_helpers::*;

/// A single operation within a batch request.
///
/// Symbol-based operations accept `symbolNames` (array of strings) to process multiple
/// symbols in one operation. Results are combined.
#[derive(Debug, Deserialize, rmcp::schemars::JsonSchema)]
#[serde(tag = "operation", rename_all = "snake_case")]
pub enum Operation {
    // ── LSP: symbol-based ─────────────────────────────────────────────────
    /// Get symbol definition source code. Accepts multiple symbol names.
    Definition {
        #[serde(rename = "symbolNames", deserialize_with = "deserialize_string_or_vec")]
        symbol_names: Vec<String>,
        language: Option<String>,
        /// DSL cd context for directory-walk fallback when workspace_symbol fails.
        #[serde(skip)]
        search_dir: Option<String>,
    },
    /// Find all references to symbols.
    References {
        #[serde(rename = "symbolNames", deserialize_with = "deserialize_string_or_vec")]
        symbol_names: Vec<String>,
        language: Option<String>,
        #[serde(skip)]
        search_dir: Option<String>,
    },
    /// Get the doc comment/docstring of symbols.
    Docstring {
        #[serde(rename = "symbolNames", deserialize_with = "deserialize_string_or_vec")]
        symbol_names: Vec<String>,
        language: Option<String>,
        #[serde(skip)]
        search_dir: Option<String>,
    },
    /// Get the source body of symbols.
    Body {
        #[serde(rename = "symbolNames", deserialize_with = "deserialize_string_or_vec")]
        symbol_names: Vec<String>,
        language: Option<String>,
        #[serde(skip)]
        search_dir: Option<String>,
    },
    /// Find callers (incoming calls) of symbols.
    Callers {
        #[serde(rename = "symbolNames", deserialize_with = "deserialize_string_or_vec")]
        symbol_names: Vec<String>,
        language: Option<String>,
        #[serde(skip)]
        search_dir: Option<String>,
    },
    /// Find callees (outgoing calls) from symbols.
    Callees {
        #[serde(rename = "symbolNames", deserialize_with = "deserialize_string_or_vec")]
        symbol_names: Vec<String>,
        language: Option<String>,
        #[serde(skip)]
        search_dir: Option<String>,
    },
    /// Find all impl blocks for a type (Rust-specific).
    Impls {
        #[serde(rename = "symbolNames", deserialize_with = "deserialize_string_or_vec")]
        symbol_names: Vec<String>,
        language: Option<String>,
        #[serde(skip)]
        search_dir: Option<String>,
    },

    // ── LSP: file-based ───────────────────────────────────────────────────
    /// Get diagnostics for a file.
    Diagnostics {
        #[serde(rename = "filePath")]
        file_path: String,
        #[serde(rename = "contextLines", default = "default_context_lines")]
        context_lines: usize,
        #[serde(rename = "showLineNumbers", default = "default_true")]
        show_line_numbers: bool,
        language: Option<String>,
    },
    /// Get hover info at a position.
    Hover {
        #[serde(rename = "filePath")]
        file_path: String,
        line: u32,
        column: u32,
        language: Option<String>,
    },
    /// Rename a symbol at a position.
    RenameSymbol {
        #[serde(rename = "filePath")]
        file_path: String,
        line: u32,
        column: u32,
        #[serde(rename = "newName")]
        new_name: String,
        language: Option<String>,
    },
    /// List symbols in a file as a tree.
    ListSymbols {
        #[serde(rename = "filePath")]
        file_path: String,
        #[serde(rename = "maxDepth", default = "default_max_depth")]
        max_depth: usize,
        language: Option<String>,
    },
    /// Get available code actions at a position (simple).
    CodeActions {
        #[serde(rename = "filePath")]
        file_path: String,
        line: u32,
        column: u32,
        language: Option<String>,
    },
    /// Request code actions with range and kind filtering.
    CodeAction {
        #[serde(rename = "filePath")]
        file_path: String,
        /// Line number (1-indexed)
        line: u32,
        /// Column number (1-indexed)
        column: u32,
        /// End line (1-indexed, defaults to same as line)
        #[serde(rename = "endLine")]
        end_line: Option<u32>,
        /// End column (1-indexed, defaults to same as column)
        #[serde(rename = "endColumn")]
        end_column: Option<u32>,
        /// Optional: only return actions of these kinds (e.g. "refactor", "quickfix")
        #[serde(default)]
        kinds: Vec<String>,
        language: Option<String>,
    },
    /// Apply a code action by index (from a previous code_actions call).
    ApplyCodeAction {
        #[serde(rename = "filePath")]
        file_path: String,
        line: u32,
        column: u32,
        /// 0-based index into the code actions list
        index: usize,
        language: Option<String>,
    },
    /// Format a file.
    Format {
        #[serde(rename = "filePath")]
        file_path: String,
        language: Option<String>,
    },
    /// Read file contents with optional line range.
    ReadFile {
        #[serde(rename = "filePath")]
        file_path: String,
        /// Start line (1-indexed, 0 = from beginning).
        #[serde(rename = "startLine", default)]
        start_line: usize,
        /// End line (1-indexed, 0 = auto).
        #[serde(rename = "endLine", default)]
        end_line: usize,
    },
    /// Search workspace files for a text pattern.
    Grep {
        pattern: String,
        /// Optional directory to scope the search.
        #[serde(rename = "searchDir")]
        search_dir: Option<String>,
    },
    /// List source files in a directory (like ls for code).
    ListDir {
        #[serde(rename = "dirPath")]
        dir_path: String,
        #[serde(rename = "maxDepth", default = "default_one")]
        max_depth: usize,
    },
    /// Edit a symbol's body/signature/docs with indentation normalization.
    Edit {
        /// Edit types: "body", "signature", "docs", "file"
        #[serde(rename = "editTypes")]
        edit_types: Vec<String>,
        path: String,
        #[serde(rename = "symbolName", default)]
        symbol_name: String,
        #[serde(rename = "newContent")]
        new_content: String,
        #[serde(rename = "searchDir", skip)]
        search_dir: Option<String>,
    },
    /// Apply a pending edit after disambiguation.
    ///
    /// Forms:
    /// - `apply_edit <id>` — confirm with stored args
    /// - `apply_edit <id> [types]` — override edit types, keep stored path/symbol
    /// - `apply_edit <id> <path> <symbol>` — correct location
    ApplyEdit {
        #[serde(rename = "editId")]
        edit_id: String,
        /// Override path (None = use stored).
        path: Option<String>,
        /// Override symbol (None = use stored).
        #[serde(rename = "symbolName")]
        symbol_name: Option<String>,
        /// Override edit types (None = use stored).
        #[serde(rename = "editTypes")]
        edit_types: Option<Vec<String>>,
    },
    /// Undo a previously applied edit.
    Undo {
        #[serde(rename = "undoId")]
        undo_id: String,
    },
    /// Targeted range edit within a symbol using before/after context anchors.
    EditRange {
        path: String,
        #[serde(rename = "symbolName", default)]
        symbol_name: String,
        /// Context lines before the region to replace (None = from start).
        #[serde(rename = "beforeCtx")]
        before_ctx: Option<String>,
        /// Context lines after the region to replace (None = to end).
        #[serde(rename = "afterCtx")]
        after_ctx: Option<String>,
        #[serde(rename = "newContent")]
        new_content: String,
        #[serde(rename = "searchDir", skip)]
        search_dir: Option<String>,
    },
    /// Send a raw LSP request (for debugging/development).
    RawLspRequest {
        method: String,
        params: serde_json::Value,
        language: String,
    },

    // ── Background processes & triggers ───────────────────────────────────
    StartProcess {
        name: String,
        command: String,
        #[serde(default)]
        args: Vec<String>,
        group: Option<String>,
    },
    StopProcess {
        name: String,
    },
    SearchProcessOutput {
        name: Option<String>,
        group: Option<String>,
        pattern: String,
    },
    DefineTrigger {
        name: String,
        pattern: String,
        #[serde(rename = "linesBefore", default)]
        lines_before: usize,
        #[serde(rename = "linesAfter", default = "default_trigger_lines_after")]
        lines_after: usize,
        #[serde(rename = "timeoutMs", default = "default_trigger_timeout")]
        timeout_ms: u64,
        group: Option<String>,
    },
    AwaitTrigger {
        name: String,
    },

    // ── Task management ──────────────────────────────────────────────────
    SetTask {
        name: String,
        description: String,
    },
    UpdateTask {
        name: String,
        #[serde(rename = "description")]
        new_description: Option<String>,
        #[serde(rename = "appendDescription")]
        append_description: Option<String>,
        completed: Option<bool>,
    },
    AddSubtask {
        #[serde(rename = "taskName")]
        task_name: String,
        #[serde(rename = "subtaskName")]
        subtask_name: String,
        description: String,
    },
    CompleteTask {
        name: String,
    },
    CompleteSubtask {
        #[serde(rename = "taskName")]
        task_name: String,
        #[serde(rename = "subtaskName")]
        subtask_name: String,
    },
    ListTasks {
        #[serde(rename = "includeCompleted", default)]
        include_completed: bool,
    },
    ListSubtasks {
        #[serde(rename = "taskName")]
        task_name: String,
        #[serde(rename = "includeCompleted", default)]
        include_completed: bool,
    },

    // ── Misc ─────────────────────────────────────────────────────────────
    /// Block until a human sends a message via Unix socket IPC.
    RequestHumanMessage,
    /// Show workspace subprojects and standalone files.
    WorkspaceInfo,
}

// ── result type ───────────────────────────────────────────────────────────────

/// Result of a single operation.
#[derive(Debug, Serialize)]
pub struct OperationResult {
    pub operation: String,
    pub success: bool,
    pub output: String,
}
