//! Operation types for batch execution.
//!
//! Defines the [`Operation`] enum (all supported DSL operations), serde helpers,
//! and the [`OperationResult`] type returned by each executed operation.

use serde::{Deserialize, Serialize};

/// A single operation within a batch request.
///
/// Symbol-based operations accept `symbolNames` (array of strings) to process multiple
/// symbols in one operation. Results are combined.
#[derive(Debug, Deserialize, rmcp::schemars::JsonSchema)]
#[serde(tag = "operation", rename_all = "snake_case")]
pub enum Operation {
    /// Get symbol definition source code. Accepts multiple symbol names.
    Definition {
        /// Symbol names to look up (e.g. ['MyType', 'MyType.method'])
        #[serde(rename = "symbolNames", deserialize_with = "deserialize_string_or_vec")]
        symbol_names: Vec<String>,
        /// Optional language to target a specific LSP
        language: Option<String>,
    },
    /// Find all references to symbols. Accepts multiple symbol names.
    References {
        /// Symbol names to search for
        #[serde(rename = "symbolNames", deserialize_with = "deserialize_string_or_vec")]
        symbol_names: Vec<String>,
        /// Optional language to target a specific LSP
        language: Option<String>,
    },
    /// Get diagnostics for a file
    Diagnostics {
        /// Path to the file
        #[serde(rename = "filePath")]
        file_path: String,
        /// Context lines around each diagnostic (default 5)
        #[serde(rename = "contextLines", default = "default_context_lines")]
        context_lines: usize,
        /// Show line numbers in output (default true)
        #[serde(rename = "showLineNumbers", default = "default_true")]
        show_line_numbers: bool,
        /// Optional language to target a specific LSP
        language: Option<String>,
    },
    /// Get hover info at a position
    Hover {
        /// Path to the file
        #[serde(rename = "filePath")]
        file_path: String,
        /// Line number (1-indexed)
        line: u32,
        /// Column number (1-indexed)
        column: u32,
        /// Optional language to target a specific LSP
        language: Option<String>,
    },
    /// Rename a symbol at a position
    RenameSymbol {
        /// Path to the file containing the symbol
        #[serde(rename = "filePath")]
        file_path: String,
        /// Line number (1-indexed)
        line: u32,
        /// Column number (1-indexed)
        column: u32,
        /// The new name for the symbol
        #[serde(rename = "newName")]
        new_name: String,
        /// Optional language to target a specific LSP
        language: Option<String>,
    },
    /// List symbols in a file as a tree (like ls for code structure)
    ListSymbols {
        /// Path to the file
        #[serde(rename = "filePath")]
        file_path: String,
        /// Max depth of symbol tree (default 3)
        #[serde(rename = "maxDepth", default = "default_max_depth")]
        max_depth: usize,
        /// Optional language to target a specific LSP
        language: Option<String>,
    },
    /// Get the doc comment/docstring of symbols. Accepts multiple symbol names.
    Docstring {
        /// Symbol names to get docstrings for
        #[serde(rename = "symbolNames", deserialize_with = "deserialize_string_or_vec")]
        symbol_names: Vec<String>,
        /// Optional language to target a specific LSP
        language: Option<String>,
    },
    /// Get the source body of symbols. Accepts multiple symbol names.
    Body {
        /// Symbol names to get bodies for
        #[serde(rename = "symbolNames", deserialize_with = "deserialize_string_or_vec")]
        symbol_names: Vec<String>,
        /// Optional language to target a specific LSP
        language: Option<String>,
    },
    /// Find all impl blocks for a type (Rust-specific)
    Impls {
        /// Type name to find implementations for
        #[serde(rename = "symbolNames", deserialize_with = "deserialize_string_or_vec")]
        symbol_names: Vec<String>,
        /// Optional language to target a specific LSP
        language: Option<String>,
    },
    /// Send a raw LSP request and return the JSON response (for debugging/development)
    RawLspRequest {
        /// The LSP method (e.g. "textDocument/completion")
        method: String,
        /// The JSON params for the request
        params: serde_json::Value,
        /// Language to target a specific LSP server
        language: String,
    },
    /// Start a named background process
    StartProcess {
        /// Unique name for this process
        name: String,
        /// Command to run
        command: String,
        /// Command arguments
        #[serde(default)]
        args: Vec<String>,
        /// Optional group name (triggers attached to this group auto-activate)
        group: Option<String>,
    },
    /// Stop a background process by name
    StopProcess {
        /// Name of the process to stop
        name: String,
    },
    /// Search background process output by name/group and pattern
    SearchProcessOutput {
        /// Process name to search (optional if group is set)
        name: Option<String>,
        /// Group name to search (optional if name is set)
        group: Option<String>,
        /// Substring pattern to search for
        pattern: String,
    },
    /// Define or update a trigger on background process output
    DefineTrigger {
        /// Unique trigger name
        name: String,
        /// Substring pattern to match
        pattern: String,
        /// Lines of context before the match (default 0)
        #[serde(rename = "linesBefore", default)]
        lines_before: usize,
        /// Lines of context after the match (default 5)
        #[serde(rename = "linesAfter", default = "default_trigger_lines_after")]
        lines_after: usize,
        /// Timeout in ms for collecting after-lines (default 3000)
        #[serde(rename = "timeoutMs", default = "default_trigger_timeout")]
        timeout_ms: u64,
        /// Auto-attach to processes started with this group
        group: Option<String>,
    },
    /// Wait for a named trigger to fire
    AwaitTrigger {
        /// Trigger name to wait for
        name: String,
    },
    /// Request code actions (refactorings, quick-fixes) at a position
    CodeAction {
        /// Path to the file
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
        /// Optional language to target a specific LSP
        language: Option<String>,
    },
    /// Show workspace structure: sub-projects, standalone files
    WorkspaceInfo,
    /// Block until a human sends a message via the Unix socket IPC.
    RequestHumanMessage,
    /// Create or replace a named task
    SetTask {
        /// Unique task name
        name: String,
        /// Task description / notes
        description: String,
    },
    /// Update an existing task's description and/or completion status
    UpdateTask {
        /// Task name
        name: String,
        /// Replace description with this (mutually exclusive with appendDescription)
        #[serde(rename = "description")]
        new_description: Option<String>,
        /// Append this text to the existing description
        #[serde(rename = "appendDescription")]
        append_description: Option<String>,
        /// Mark task as completed (true) or reopen it (false)
        completed: Option<bool>,
    },
    /// Add or update a subtask within a task
    AddSubtask {
        /// Parent task name
        #[serde(rename = "taskName")]
        task_name: String,
        /// Subtask name
        #[serde(rename = "subtaskName")]
        subtask_name: String,
        /// Subtask description
        description: String,
    },
    /// Mark a task as completed
    CompleteTask {
        /// Task name to mark done
        name: String,
    },
    /// Mark a subtask as completed
    CompleteSubtask {
        /// Parent task name
        #[serde(rename = "taskName")]
        task_name: String,
        /// Subtask name to mark done
        #[serde(rename = "subtaskName")]
        subtask_name: String,
    },
    /// List tasks (default: only pending tasks)
    ListTasks {
        /// Include completed tasks in output (default false)
        #[serde(rename = "includeCompleted", default)]
        include_completed: bool,
    },
    /// List subtasks of a task (default: only pending subtasks)
    ListSubtasks {
        /// Task name
        #[serde(rename = "taskName")]
        task_name: String,
        /// Include completed subtasks in output (default false)
        #[serde(rename = "includeCompleted", default)]
        include_completed: bool,
    },
}

// ── serde helpers ─────────────────────────────────────────────────────────────

fn default_max_depth() -> usize {
    3
}

fn default_trigger_lines_after() -> usize {
    5
}

fn default_trigger_timeout() -> u64 {
    3000
}

fn default_context_lines() -> usize {
    5
}

fn default_true() -> bool {
    true
}

/// Deserialize either a single string or a vec of strings into `Vec<String>`.
pub fn deserialize_string_or_vec<'de, D>(deserializer: D) -> Result<Vec<String>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    use serde::de;

    struct StringOrVec;

    impl<'de> de::Visitor<'de> for StringOrVec {
        type Value = Vec<String>;

        fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
            f.write_str("a string or array of strings")
        }

        fn visit_str<E: de::Error>(self, v: &str) -> Result<Self::Value, E> {
            Ok(vec![v.to_string()])
        }

        fn visit_string<E: de::Error>(self, v: String) -> Result<Self::Value, E> {
            Ok(vec![v])
        }

        fn visit_seq<A: de::SeqAccess<'de>>(self, mut seq: A) -> Result<Self::Value, A::Error> {
            let mut vec = Vec::new();
            while let Some(s) = seq.next_element()? {
                vec.push(s);
            }
            Ok(vec)
        }
    }

    deserializer.deserialize_any(StringOrVec)
}

// ── result type ───────────────────────────────────────────────────────────────

/// Result of a single operation.
#[derive(Debug, Serialize)]
pub struct OperationResult {
    pub operation: String,
    pub success: bool,
    pub output: String,
}
