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
    /// Block until a human sends a message via the Unix socket IPC.
    /// Use this instead of ending the session when you need human input.
    RequestHumanMessage,
}

fn default_max_depth() -> usize {
    3
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
    operations: Vec<Operation>,
) -> Vec<OperationResult> {
    let futures: Vec<_> = operations
        .into_iter()
        .map(|op| {
            let manager = manager.clone();
            let bus = message_bus.clone();
            tokio::spawn(async move { execute_one(&manager, &bus, op).await })
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

        Operation::RequestHumanMessage => {
            let msg = message_bus.wait_for_message().await;
            OperationResult {
                operation: "request_human_message".into(),
                success: true,
                output: msg,
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
