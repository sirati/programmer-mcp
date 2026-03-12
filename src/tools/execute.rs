//! Core operation execution.
//!
//! `execute_batch` runs a list of operations concurrently.
//! `execute_one` dispatches a single `Operation`, delegating process/trigger and
//! task sub-groups to their dedicated modules.

use std::sync::Arc;

use crate::background::BackgroundManager;
use crate::ipc::HumanMessageBus;
use crate::lsp::manager::LspManager;

use super::exec_helpers::{execute_multi_symbol, execute_on_first};
use super::json_util::{format_compact_json, strip_json_noise};
use super::operation::{Operation, OperationResult};
use super::{definition, diagnostics, hover, impls, references, rename, symbol_info, symbol_list};
use super::{process_ops, task_ops};

// ── public API ────────────────────────────────────────────────────────────────

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

// ── dispatch ──────────────────────────────────────────────────────────────────

async fn execute_one(
    manager: &LspManager,
    message_bus: &HumanMessageBus,
    background: Arc<BackgroundManager>,
    op: Operation,
) -> OperationResult {
    match op {
        // ── LSP: symbol-based ─────────────────────────────────────────────────
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

        // ── LSP: file-based ───────────────────────────────────────────────────
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

        // ── LSP: raw request ──────────────────────────────────────────────────
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

        // ── Human message ─────────────────────────────────────────────────────
        Operation::RequestHumanMessage => {
            let msg = message_bus.wait_for_message().await;
            OperationResult {
                operation: "request_human_message".into(),
                success: true,
                output: msg,
            }
        }

        // ── Process / trigger operations ──────────────────────────────────────
        op @ (Operation::StartProcess { .. }
        | Operation::StopProcess { .. }
        | Operation::SearchProcessOutput { .. }
        | Operation::DefineTrigger { .. }
        | Operation::AwaitTrigger { .. }) => process_ops::execute(op, &background).await,

        // ── Task management operations ────────────────────────────────────────
        op @ (Operation::SetTask { .. }
        | Operation::UpdateTask { .. }
        | Operation::AddSubtask { .. }
        | Operation::CompleteTask { .. }
        | Operation::CompleteSubtask { .. }
        | Operation::ListTasks { .. }
        | Operation::ListSubtasks { .. }) => task_ops::execute(op, &background).await,
    }
}
