//! Client dispatch helpers for LSP operation execution.
//!
//! Provides `execute_on_first` and `execute_multi_symbol`
//! which wrap LSP client calls into `OperationResult` values.

use std::sync::Arc;

use crate::lsp::client::{LspClient, LspClientError};

use super::operation::OperationResult;

// ── client helpers ────────────────────────────────────────────────────────────

/// Execute on the first matching client (for file-based operations).
pub async fn execute_on_first<F, Fut>(
    op_name: &str,
    clients: Vec<&Arc<LspClient>>,
    f: F,
) -> OperationResult
where
    F: Fn(Arc<LspClient>) -> Fut,
    Fut: std::future::Future<Output = Result<String, LspClientError>>,
{
    if clients.is_empty() {
        return no_client(op_name);
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
/// `search_dir` is the DSL cd context passed through for directory-walk fallback.
pub async fn execute_multi_symbol<F, Fut>(
    op_name: &str,
    clients: Vec<&Arc<LspClient>>,
    names: &[String],
    search_dir: Option<&str>,
    f: F,
) -> OperationResult
where
    F: Fn(Arc<LspClient>, String, Option<String>) -> Fut,
    Fut: std::future::Future<Output = Result<String, LspClientError>>,
{
    if clients.is_empty() {
        return no_client(op_name);
    }

    let mut futures = Vec::new();
    for name in names {
        for client in &clients {
            futures.push(f(
                (*client).clone(),
                name.clone(),
                search_dir.map(String::from),
            ));
        }
    }

    let results = futures::future::join_all(futures).await;

    let mut found_parts = Vec::new();
    let mut not_found_parts = Vec::new();
    let mut seen = std::collections::HashSet::new();

    for result in results {
        match result {
            Ok(text) if !text.is_empty() => {
                // Classify: "not found" vs real results
                let trimmed = text.trim().to_string();
                if is_not_found_msg(&trimmed) {
                    not_found_parts.push(trimmed);
                } else if seen.insert(trimmed.clone()) {
                    found_parts.push(trimmed);
                }
            }
            Ok(_) => {}
            Err(e) => tracing::debug!(op = op_name, "client error: {e}"),
        }
    }

    // Only include "not found" messages if no real results were found
    let parts = if found_parts.is_empty() {
        // Deduplicate not-found messages too
        not_found_parts.sort();
        not_found_parts.dedup();
        not_found_parts
    } else {
        found_parts
    };

    OperationResult {
        operation: op_name.into(),
        success: !parts.is_empty() && parts.iter().any(|p| !p.contains("not found")),
        output: if parts.is_empty() {
            format!("no results for {op_name}")
        } else {
            parts.join("\n\n---\n\n")
        },
    }
}

// ── shared error ──────────────────────────────────────────────────────────────

/// Check if a result string is a "not found" / "no results" type message
/// (as opposed to a real result that happens to mention "not found").
fn is_not_found_msg(text: &str) -> bool {
    fn is_not_found_line(l: &str) -> bool {
        l.contains("not found")
            || l.starts_with("No ")
            || l.starts_with("Did you mean")
            || l.trim().is_empty()
    }
    if !text.contains('\n') {
        return text.contains("not found") || text.starts_with("No ");
    }
    let lines: Vec<&str> = text.lines().collect();
    lines.len() <= 4 && lines.iter().all(|l| is_not_found_line(l))
}

fn no_client(op_name: &str) -> OperationResult {
    OperationResult {
        operation: op_name.into(),
        success: false,
        output: "no LSP client available for this operation".into(),
    }
}
