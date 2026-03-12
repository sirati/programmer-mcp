//! Client dispatch helpers for LSP operation execution.
//!
//! Provides `execute_on_clients`, `execute_on_first`, and `execute_multi_symbol`
//! which wrap LSP client calls into `OperationResult` values.

use std::sync::Arc;

use crate::lsp::client::{LspClient, LspClientError};

use super::operation::OperationResult;

// ── multi-client helpers ──────────────────────────────────────────────────────

/// Execute on all clients and merge results (for symbol-based operations).
pub async fn execute_on_clients<F, Fut>(
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
            Err(e) => tracing::debug!(op = op_name, "client error: {e}"),
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
pub async fn execute_multi_symbol<F, Fut>(
    op_name: &str,
    clients: Vec<&Arc<LspClient>>,
    names: &[String],
    f: F,
) -> OperationResult
where
    F: Fn(Arc<LspClient>, String) -> Fut,
    Fut: std::future::Future<Output = Result<String, LspClientError>>,
{
    if clients.is_empty() {
        return no_client(op_name);
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
            Err(e) => tracing::debug!(op = op_name, "client error: {e}"),
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

// ── shared error ──────────────────────────────────────────────────────────────

fn no_client(op_name: &str) -> OperationResult {
    OperationResult {
        operation: op_name.into(),
        success: false,
        output: "no LSP client available for this operation".into(),
    }
}
