pub mod definition;
pub mod diagnostics;
pub mod formatting;
pub mod hover;
pub mod impls;
pub mod language_specific;
pub mod references;
pub mod rename;
pub mod symbol_info;
pub mod symbol_list;
pub mod symbol_search;

use std::sync::Arc;

use rmcp::schemars;
use serde::{Deserialize, Serialize};

use crate::background::BackgroundManager;
use crate::ipc::HumanMessageBus;
use crate::lsp::manager::LspManager;

/// A single operation within a batch request.
///
/// Symbol-based operations accept `symbolNames` (array of names) to process multiple
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
    /// Find all impl blocks for a type (Rust-specific: lists `impl Type` and `impl Trait for Type`)
    Impls {
        /// Type name to find implementations for
        #[serde(rename = "symbolNames", deserialize_with = "deserialize_string_or_vec")]
        symbol_names: Vec<String>,
        /// Optional language to target a specific LSP
        language: Option<String>,
    },
    /// Send a raw LSP request and return the JSON response (for debugging/development)
    RawLspRequest {
        /// The LSP method (e.g. "textDocument/completion", "textDocument/signatureHelp")
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
    /// Block until a human sends a message via the Unix socket IPC.
    /// Use this instead of ending the session when you need human input.
    RequestHumanMessage,
    /// Create or replace a named task (saved to .programmer-mcp/tasks/{name}.json)
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

fn default_max_depth() -> usize {
    3
}

fn default_trigger_lines_after() -> usize {
    5
}

fn default_trigger_timeout() -> u64 {
    3000
}

/// Deserialize either a single string or a vec of strings into Vec<String>.
fn deserialize_string_or_vec<'de, D>(deserializer: D) -> Result<Vec<String>, D::Error>
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

fn default_context_lines() -> usize {
    5
}

fn default_true() -> bool {
    true
}

/// Result of a single operation.
#[derive(Debug, Serialize)]
pub struct OperationResult {
    pub operation: String,
    pub success: bool,
    pub output: String,
}

/// Execute a batch of operations concurrently.
pub async fn execute_batch(
    manager: &Arc<LspManager>,
    message_bus: &Arc<HumanMessageBus>,
    background: &Arc<BackgroundManager>,
    operations: Vec<Operation>,
) -> Vec<OperationResult> {
    let futures: Vec<_> = operations
        .into_iter()
        .map(|op| {
            let manager = manager.clone();
            let bus = message_bus.clone();
            let bg = background.clone();
            tokio::spawn(async move { execute_one(&manager, &bus, bg, op).await })
        })
        .collect();

    let mut results = Vec::with_capacity(futures.len());
    for future in futures {
        match future.await {
            Ok(result) => results.push(result),
            Err(e) => results.push(OperationResult {
                operation: "unknown".into(),
                success: false,
                output: format!("task panicked: {e}"),
            }),
        }
    }

    results
}

async fn execute_one(
    manager: &LspManager,
    message_bus: &HumanMessageBus,
    background: Arc<BackgroundManager>,
    op: Operation,
) -> OperationResult {
    match op {
        Operation::Definition {
            symbol_names,
            language,
        } => {
            let clients = manager.resolve(language.as_deref(), None);
            execute_multi_symbol(
                "definition",
                clients,
                &symbol_names,
                |client, name| async move { definition::read_definition(&client, &name).await },
            )
            .await
        }

        Operation::References {
            symbol_names,
            language,
        } => {
            let clients = manager.resolve(language.as_deref(), None);
            execute_multi_symbol(
                "references",
                clients,
                &symbol_names,
                |client, name| async move { references::find_references(&client, &name, 5).await },
            )
            .await
        }

        Operation::Diagnostics {
            file_path,
            context_lines,
            show_line_numbers,
            language,
        } => {
            let clients = manager.resolve(language.as_deref(), Some(&file_path));
            execute_on_first("diagnostics", clients, |client| {
                let path = file_path.clone();
                async move {
                    diagnostics::get_diagnostics(&client, &path, context_lines, show_line_numbers)
                        .await
                }
            })
            .await
        }

        Operation::Hover {
            file_path,
            line,
            column,
            language,
        } => {
            let clients = manager.resolve(language.as_deref(), Some(&file_path));
            execute_on_first("hover", clients, |client| {
                let path = file_path.clone();
                async move { hover::get_hover_info(&client, &path, line, column).await }
            })
            .await
        }

        Operation::RenameSymbol {
            file_path,
            line,
            column,
            new_name,
            language,
        } => {
            let clients = manager.resolve(language.as_deref(), Some(&file_path));
            execute_on_first("rename_symbol", clients, |client| {
                let path = file_path.clone();
                let name = new_name.clone();
                async move { rename::rename_symbol(&client, &path, line, column, &name).await }
            })
            .await
        }

        Operation::ListSymbols {
            file_path,
            max_depth,
            language,
        } => {
            let clients = manager.resolve(language.as_deref(), Some(&file_path));
            execute_on_first("list_symbols", clients, |client| {
                let path = file_path.clone();
                async move { symbol_list::list_symbols(&client, &path, max_depth).await }
            })
            .await
        }

        Operation::Docstring {
            symbol_names,
            language,
        } => {
            let clients = manager.resolve(language.as_deref(), None);
            execute_multi_symbol(
                "docstring",
                clients,
                &symbol_names,
                |client, name| async move { symbol_info::get_docstring(&client, &name).await },
            )
            .await
        }

        Operation::Body {
            symbol_names,
            language,
        } => {
            let clients = manager.resolve(language.as_deref(), None);
            execute_multi_symbol("body", clients, &symbol_names, |client, name| async move {
                symbol_info::get_body(&client, &name).await
            })
            .await
        }

        Operation::Impls {
            symbol_names,
            language,
        } => {
            let clients = manager.resolve(language.as_deref(), None);
            execute_multi_symbol("impls", clients, &symbol_names, |client, name| async move {
                impls::find_impls(&client, &name).await
            })
            .await
        }

        Operation::RawLspRequest {
            method,
            params,
            language,
        } => {
            let clients = manager.resolve(Some(&language), None);
            execute_on_first("raw_lsp_request", clients, |client| {
                let m = method.clone();
                let p = params.clone();
                async move {
                    let result = client.raw_request(&m, p).await?;
                    Ok(format_compact_json(&strip_json_noise(result)))
                }
            })
            .await
        }

        Operation::StartProcess {
            name,
            command,
            args,
            group,
        } => {
            // Start process
            let result = background.processes.lock().await.start(
                name.clone(),
                group.clone(),
                &command,
                &args,
            );

            // If group is set, auto-attach matching triggers
            if let (Ok(()), Some(ref grp)) = (&result, &group) {
                let triggers = background.triggers.lock().await;
                let group_triggers: Vec<_> = triggers
                    .triggers_for_group(grp)
                    .into_iter()
                    .cloned()
                    .collect();
                drop(triggers);

                for config in group_triggers {
                    let bg = background.clone();
                    let proc_name = name.clone();
                    tokio::spawn(async move {
                        run_trigger_scanner(&bg, &proc_name, &config).await;
                    });
                }
            }

            match result {
                Ok(()) => OperationResult {
                    operation: "start_process".into(),
                    success: true,
                    output: String::new(),
                },
                Err(e) => OperationResult {
                    operation: "start_process".into(),
                    success: false,
                    output: e,
                },
            }
        }

        Operation::StopProcess { name } => match background.processes.lock().await.stop(&name) {
            Ok(()) => OperationResult {
                operation: "stop_process".into(),
                success: true,
                output: String::new(),
            },
            Err(e) => OperationResult {
                operation: "stop_process".into(),
                success: false,
                output: e,
            },
        },

        Operation::SearchProcessOutput {
            name,
            group,
            pattern,
        } => {
            let procs = background.processes.lock().await;
            let results = procs.search_output(name.as_deref(), group.as_deref(), &pattern);
            let output = if results.is_empty() {
                "no matches".into()
            } else {
                results
                    .into_iter()
                    .map(|(proc_name, lines)| format!("--- {proc_name} ---\n{}", lines.join("\n")))
                    .collect::<Vec<_>>()
                    .join("\n\n")
            };
            OperationResult {
                operation: "search_process_output".into(),
                success: true,
                output,
            }
        }

        Operation::DefineTrigger {
            name,
            pattern,
            lines_before,
            lines_after,
            timeout_ms,
            group,
        } => {
            let config = crate::background::trigger::TriggerConfig {
                name,
                pattern,
                lines_before,
                lines_after,
                timeout_ms,
                group,
            };
            match background.triggers.lock().await.define(config) {
                Ok(()) => OperationResult {
                    operation: "define_trigger".into(),
                    success: true,
                    output: String::new(),
                },
                Err(e) => OperationResult {
                    operation: "define_trigger".into(),
                    success: false,
                    output: e,
                },
            }
        }

        Operation::AwaitTrigger { name } => {
            // Check if already fired
            {
                let triggers = background.triggers.lock().await;
                if let Some(result) = triggers
                    .pending_results
                    .iter()
                    .find(|r| r.trigger_name == name)
                {
                    let output = result.to_string();
                    return OperationResult {
                        operation: "await_trigger".into(),
                        success: true,
                        output,
                    };
                }
            }

            // Get timeout from trigger config
            let timeout_ms = background
                .triggers
                .lock()
                .await
                .get(&name)
                .map(|c| c.timeout_ms)
                .unwrap_or(30000);

            // Wait for it to fire
            let mut rx = background.triggers.lock().await.subscribe();
            let deadline =
                tokio::time::Instant::now() + std::time::Duration::from_millis(timeout_ms);

            loop {
                if tokio::time::Instant::now() >= deadline {
                    return OperationResult {
                        operation: "await_trigger".into(),
                        success: true,
                        output: format!("trigger '{name}' timed out after {timeout_ms}ms"),
                    };
                }

                let timeout = tokio::time::timeout_at(deadline, rx.changed()).await;
                match timeout {
                    Ok(Ok(())) => {
                        let triggers = background.triggers.lock().await;
                        if let Some(result) = triggers
                            .pending_results
                            .iter()
                            .find(|r| r.trigger_name == name)
                        {
                            return OperationResult {
                                operation: "await_trigger".into(),
                                success: true,
                                output: result.to_string(),
                            };
                        }
                    }
                    _ => {
                        return OperationResult {
                            operation: "await_trigger".into(),
                            success: true,
                            output: format!("trigger '{name}' timed out after {timeout_ms}ms"),
                        };
                    }
                }
            }
        }

        Operation::RequestHumanMessage => {
            let msg = message_bus.wait_for_message().await;
            OperationResult {
                operation: "request_human_message".into(),
                success: true,
                output: msg,
            }
        }

        Operation::SetTask { name, description } => {
            match background.tasks.lock().await.set_task(&name, &description) {
                Ok(()) => OperationResult {
                    operation: "set_task".into(),
                    success: true,
                    output: format!("task '{name}' set"),
                },
                Err(e) => OperationResult {
                    operation: "set_task".into(),
                    success: false,
                    output: e,
                },
            }
        }

        Operation::UpdateTask {
            name,
            new_description,
            append_description,
            completed,
        } => {
            match background.tasks.lock().await.update_task(
                &name,
                new_description.as_deref(),
                append_description.as_deref(),
                completed,
            ) {
                Ok(()) => OperationResult {
                    operation: "update_task".into(),
                    success: true,
                    output: format!("task '{name}' updated"),
                },
                Err(e) => OperationResult {
                    operation: "update_task".into(),
                    success: false,
                    output: e,
                },
            }
        }

        Operation::AddSubtask {
            task_name,
            subtask_name,
            description,
        } => {
            match background
                .tasks
                .lock()
                .await
                .add_subtask(&task_name, &subtask_name, &description)
            {
                Ok(()) => OperationResult {
                    operation: "add_subtask".into(),
                    success: true,
                    output: format!("subtask '{subtask_name}' added to '{task_name}'"),
                },
                Err(e) => OperationResult {
                    operation: "add_subtask".into(),
                    success: false,
                    output: e,
                },
            }
        }

        Operation::CompleteTask { name } => {
            match background.tasks.lock().await.complete_task(&name) {
                Ok(()) => OperationResult {
                    operation: "complete_task".into(),
                    success: true,
                    output: format!("task '{name}' marked completed"),
                },
                Err(e) => OperationResult {
                    operation: "complete_task".into(),
                    success: false,
                    output: e,
                },
            }
        }

        Operation::CompleteSubtask {
            task_name,
            subtask_name,
        } => {
            match background
                .tasks
                .lock()
                .await
                .complete_subtask(&task_name, &subtask_name)
            {
                Ok(()) => OperationResult {
                    operation: "complete_subtask".into(),
                    success: true,
                    output: format!("subtask '{subtask_name}' in '{task_name}' marked completed"),
                },
                Err(e) => OperationResult {
                    operation: "complete_subtask".into(),
                    success: false,
                    output: e,
                },
            }
        }

        Operation::ListTasks { include_completed } => {
            let output = background.tasks.lock().await.list_tasks(include_completed);
            OperationResult {
                operation: "list_tasks".into(),
                success: true,
                output,
            }
        }

        Operation::ListSubtasks {
            task_name,
            include_completed,
        } => {
            let output = background
                .tasks
                .lock()
                .await
                .list_subtasks(&task_name, include_completed);
            OperationResult {
                operation: "list_subtasks".into(),
                success: true,
                output,
            }
        }
    }
}

/// Execute on all clients and merge results (for symbol-based operations).
async fn execute_on_clients<F, Fut>(
    op_name: &str,
    clients: Vec<&Arc<crate::lsp::client::LspClient>>,
    f: F,
) -> OperationResult
where
    F: Fn(Arc<crate::lsp::client::LspClient>) -> Fut,
    Fut: std::future::Future<Output = Result<String, crate::lsp::client::LspClientError>>,
{
    if clients.is_empty() {
        return OperationResult {
            operation: op_name.into(),
            success: false,
            output: "no LSP client available for this operation".into(),
        };
    }

    let futures: Vec<_> = clients.into_iter().map(|c| f(c.clone())).collect();

    let results = futures::future::join_all(futures).await;

    let mut output = String::new();
    let mut any_success = false;

    for result in results {
        match result {
            Ok(text) if !text.is_empty() => {
                any_success = true;
                output.push_str(&text);
            }
            Ok(_) => {}
            Err(e) => {
                tracing::debug!(op = op_name, "client error: {e}");
            }
        }
    }

    OperationResult {
        operation: op_name.into(),
        success: any_success,
        output: if output.is_empty() {
            format!("no results for {op_name}")
        } else {
            output
        },
    }
}

/// Execute on the first matching client (for file-based operations).
async fn execute_on_first<F, Fut>(
    op_name: &str,
    clients: Vec<&Arc<crate::lsp::client::LspClient>>,
    f: F,
) -> OperationResult
where
    F: Fn(Arc<crate::lsp::client::LspClient>) -> Fut,
    Fut: std::future::Future<Output = Result<String, crate::lsp::client::LspClientError>>,
{
    if clients.is_empty() {
        return OperationResult {
            operation: op_name.into(),
            success: false,
            output: "no LSP client available for this operation".into(),
        };
    }

    match f(clients[0].clone()).await {
        Ok(text) => OperationResult {
            operation: op_name.into(),
            success: true,
            output: text,
        },
        Err(e) => OperationResult {
            operation: op_name.into(),
            success: false,
            output: format!("{op_name} failed: {e}"),
        },
    }
}

/// Execute a symbol-based operation for multiple symbol names across all matching clients.
async fn execute_multi_symbol<F, Fut>(
    op_name: &str,
    clients: Vec<&Arc<crate::lsp::client::LspClient>>,
    names: &[String],
    f: F,
) -> OperationResult
where
    F: Fn(Arc<crate::lsp::client::LspClient>, String) -> Fut,
    Fut: std::future::Future<Output = Result<String, crate::lsp::client::LspClientError>>,
{
    if clients.is_empty() {
        return OperationResult {
            operation: op_name.into(),
            success: false,
            output: "no LSP client available for this operation".into(),
        };
    }

    let mut futures = Vec::new();
    for name in names {
        for client in &clients {
            futures.push(f((*client).clone(), name.clone()));
        }
    }

    let results = futures::future::join_all(futures).await;

    let mut parts = Vec::new();
    let mut any_success = false;
    for result in results {
        match result {
            Ok(text) if !text.is_empty() => {
                any_success = true;
                parts.push(text);
            }
            Ok(_) => {}
            Err(e) => {
                tracing::debug!(op = op_name, "client error: {e}");
            }
        }
    }

    OperationResult {
        operation: op_name.into(),
        success: any_success,
        output: if parts.is_empty() {
            format!("no results for {op_name}")
        } else {
            parts.join("\n\n---\n\n")
        },
    }
}

/// Remove null values and empty arrays/objects from JSON to reduce noise.
fn strip_json_noise(val: serde_json::Value) -> serde_json::Value {
    match val {
        serde_json::Value::Object(map) => {
            let cleaned: serde_json::Map<String, serde_json::Value> = map
                .into_iter()
                .filter_map(|(k, v)| {
                    if v.is_null() {
                        return None;
                    }
                    let v = strip_json_noise(v);
                    if matches!(&v, serde_json::Value::Array(a) if a.is_empty()) {
                        return None;
                    }
                    if matches!(&v, serde_json::Value::Object(m) if m.is_empty()) {
                        return None;
                    }
                    Some((k, v))
                })
                .collect();
            serde_json::Value::Object(cleaned)
        }
        serde_json::Value::Array(arr) => {
            serde_json::Value::Array(arr.into_iter().map(strip_json_noise).collect())
        }
        other => other,
    }
}

/// Format JSON compactly: objects/arrays on single lines, one entry per line at top level.
fn format_compact_json(val: &serde_json::Value) -> String {
    match val {
        serde_json::Value::Array(arr) => arr
            .iter()
            .map(|v| serde_json::to_string(v).unwrap_or_else(|_| v.to_string()))
            .collect::<Vec<_>>()
            .join("\n"),
        _ => serde_json::to_string(val).unwrap_or_else(|_| val.to_string()),
    }
}

/// Spawn a task that watches a process's output and fires a trigger when the pattern matches.
async fn run_trigger_scanner(
    background: &Arc<BackgroundManager>,
    process_name: &str,
    config: &crate::background::trigger::TriggerConfig,
) {
    let (mut rx, start_line) = {
        let procs = background.processes.lock().await;
        let Some(proc) = procs.get(process_name) else {
            return;
        };
        (proc.subscribe(), proc.line_count())
    };

    let mut checked_up_to = start_line;
    let pattern = config.pattern.clone();
    let trigger_name = config.name.clone();
    let lines_before = config.lines_before;
    let lines_after = config.lines_after;
    let timeout_ms = config.timeout_ms;

    loop {
        // Wait for new output
        if rx.changed().await.is_err() {
            return; // process ended
        }

        let procs = background.processes.lock().await;
        let Some(proc) = procs.get(process_name) else {
            return;
        };

        let new_lines = proc.lines_from(checked_up_to);
        let current_count = checked_up_to + new_lines.len();

        for (i, line) in new_lines.iter().enumerate() {
            if line.contains(&pattern) {
                let match_idx = checked_up_to + i;

                // Gather context before
                let before_start = match_idx.saturating_sub(lines_before);
                let all_lines = proc.lines_from(0);

                let mut context: Vec<String> = all_lines
                    [before_start..=match_idx.min(all_lines.len().saturating_sub(1))]
                    .to_vec();

                // Need to collect after-lines; drop locks first
                drop(procs);

                // Wait briefly for after-lines
                let after_deadline =
                    tokio::time::Instant::now() + std::time::Duration::from_millis(timeout_ms);
                let mut collected_after = 0;

                while collected_after < lines_after {
                    let timeout = tokio::time::timeout_at(after_deadline, rx.changed()).await;
                    match timeout {
                        Ok(Ok(())) => {
                            let procs = background.processes.lock().await;
                            if let Some(proc) = procs.get(process_name) {
                                let after_lines = proc.lines_from(match_idx + 1 + collected_after);
                                context.extend(after_lines.iter().cloned());
                                collected_after += after_lines.len();
                            } else {
                                break;
                            }
                        }
                        _ => break, // timeout
                    }
                }

                let result = crate::background::trigger::TriggerResult {
                    trigger_name: trigger_name.clone(),
                    matched_line: line.clone(),
                    context,
                };
                background.triggers.lock().await.record_fire(result);
                return; // trigger fired, done
            }
        }

        checked_up_to = current_count;
    }
}
