//! Core operation execution.
//!
//! `execute_batch` runs a list of operations concurrently.
//! `execute_one` dispatches a single `Operation` to the appropriate sub-module.

use std::path::Path;
use std::sync::Arc;

use crate::background::BackgroundManager;
use crate::config::LengthLimits;
use crate::ipc::HumanMessageBus;
use crate::lsp::manager::LspManager;

use super::edit::{self, PendingEdits, UndoStore};
use super::execute_lsp;
use super::operation::{Operation, OperationResult};
use super::{grep, list_dir, process_ops, read_file, task_ops, workspace};

/// Execute a batch of operations concurrently.
pub async fn execute_batch(
    manager: &Arc<LspManager>,
    message_bus: &Arc<HumanMessageBus>,
    background: &Arc<BackgroundManager>,
    workspace_root: &Path,
    operations: Vec<Operation>,
    pending_edits: &PendingEdits,
    undo_store: &UndoStore,
    limits: LengthLimits,
) -> Vec<OperationResult> {
    let futures: Vec<_> = operations
        .into_iter()
        .map(|op| {
            let manager = manager.clone();
            let bus = message_bus.clone();
            let bg = background.clone();
            let ws = workspace_root.to_path_buf();
            let pe = pending_edits.clone();
            let us = undo_store.clone();
            tokio::spawn(async move {
                execute_one(&manager, &bus, bg, &ws, op, &pe, &us, &limits).await
            })
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
    workspace_root: &Path,
    op: Operation,
    pending_edits: &PendingEdits,
    undo_store: &UndoStore,
    limits: &LengthLimits,
) -> OperationResult {
    match &op {
        // LSP: symbol-based
        Operation::Definition { .. }
        | Operation::References { .. }
        | Operation::Docstring { .. }
        | Operation::Body { .. }
        | Operation::Callers { .. }
        | Operation::Callees { .. }
        | Operation::Impls { .. } => execute_lsp::execute_symbol_op(manager, op).await,

        // LSP: file-based
        Operation::Diagnostics { .. }
        | Operation::Hover { .. }
        | Operation::RenameSymbol { .. }
        | Operation::ListSymbols { .. }
        | Operation::CodeActions { .. }
        | Operation::CodeAction { .. }
        | Operation::ApplyCodeAction { .. }
        | Operation::Format { .. }
        | Operation::RawLspRequest { .. } => execute_lsp::execute_file_op(manager, op).await,

        // Human message
        Operation::RequestHumanMessage => {
            let msg = message_bus.wait_for_message().await;
            OperationResult {
                operation: "request_human_message".into(),
                success: true,
                output: msg,
            }
        }

        // Read file
        Operation::ReadFile {
            file_path,
            start_line,
            end_line,
        } => {
            let output = read_file::read_file(file_path, *start_line, *end_line, workspace_root);
            OperationResult {
                operation: "read".into(),
                success: true,
                output,
            }
        }

        // Grep search
        Operation::Grep {
            pattern,
            search_dir,
        } => {
            let output = grep::grep_workspace(pattern, search_dir.as_deref(), workspace_root);
            OperationResult {
                operation: "grep".into(),
                success: true,
                output,
            }
        }

        // Directory listing
        Operation::ListDir {
            dir_path,
            max_depth,
        } => {
            let output = list_dir::list_source_files(&dir_path, *max_depth, workspace_root);
            OperationResult {
                operation: "list_dir".into(),
                success: true,
                output,
            }
        }

        // Workspace info
        Operation::WorkspaceInfo => {
            let output = workspace::workspace_info(workspace_root);
            OperationResult {
                operation: "workspace_info".into(),
                success: true,
                output,
            }
        }

        // Edit operations (need LSP + pending edits)
        Operation::Edit {
            edit_types,
            path,
            symbol_name,
            new_content,
            search_dir,
        } => {
            use super::edit::EditType;
            use super::exec_helpers::execute_on_first;

            let parsed_types: Vec<EditType> = edit_types
                .iter()
                .filter_map(|t| EditType::from_str(t))
                .collect();

            // File-only edits don't need LSP
            if parsed_types.len() == 1 && parsed_types[0] == EditType::File {
                match edit::execute_edit_no_lsp(
                    &parsed_types,
                    path,
                    symbol_name,
                    new_content,
                    pending_edits,
                    undo_store,
                    limits,
                )
                .await
                {
                    Ok(output) => OperationResult {
                        operation: "edit".into(),
                        success: true,
                        output,
                    },
                    Err(e) => OperationResult {
                        operation: "edit".into(),
                        success: false,
                        output: format!("edit failed: {e}"),
                    },
                }
            } else {
                let clients = manager.resolve(None, Some(path));
                let pe = pending_edits.clone();
                let us = undo_store.clone();
                let lim = *limits;
                execute_on_first("edit", clients, |client| {
                    let types = parsed_types.clone();
                    let p = path.clone();
                    let sym = symbol_name.clone();
                    let content = new_content.clone();
                    let sd = search_dir.clone();
                    let pe = pe.clone();
                    let us = us.clone();
                    async move {
                        edit::execute_edit(
                            &client,
                            &types,
                            &p,
                            &sym,
                            &content,
                            sd.as_deref(),
                            &pe,
                            &us,
                            &lim,
                        )
                        .await
                    }
                })
                .await
            }
        }

        Operation::ApplyEdit {
            edit_id,
            path,
            symbol_name,
            edit_types,
        } => {
            use super::exec_helpers::execute_on_first;

            // Parse edit_types override if provided
            let types_override: Option<Vec<edit::EditType>> = edit_types.as_ref().map(|ts| {
                ts.iter()
                    .filter_map(|t| edit::EditType::from_str(t))
                    .collect()
            });

            // Determine which path to use for LSP client resolution.
            // Prefer the override path, then look up stored pending edit path.
            let resolve_path = if let Some(ref p) = path {
                Some(p.clone())
            } else {
                // Peek at stored pending edit for the path
                let map = pending_edits.lock().await;
                map.get(edit_id.as_str()).map(|pe| pe.path.clone())
            };

            let resolve_ref = resolve_path.as_deref().unwrap_or("");
            let clients = manager.resolve(None, Some(resolve_ref));
            let pe = pending_edits.clone();
            let us = undo_store.clone();

            let lim = *limits;
            execute_on_first("apply_edit", clients, |client| {
                let id = edit_id.clone();
                let p = path.clone();
                let sym = symbol_name.clone();
                let to = types_override.clone();
                let pe = pe.clone();
                let us = us.clone();
                async move {
                    edit::apply_pending_edit(
                        &client,
                        &id,
                        p.as_deref(),
                        sym.as_deref(),
                        to.as_deref(),
                        &pe,
                        &us,
                        &lim,
                    )
                    .await
                }
            })
            .await
        }

        Operation::Undo { undo_id } => match edit::execute_undo(undo_id, undo_store).await {
            Ok(output) => OperationResult {
                operation: "undo".into(),
                success: true,
                output,
            },
            Err(e) => OperationResult {
                operation: "undo".into(),
                success: false,
                output: format!("undo failed: {e}"),
            },
        },

        Operation::EditRange {
            path,
            symbol_name,
            before_ctx,
            after_ctx,
            new_content,
            search_dir,
        } => {
            use super::exec_helpers::execute_on_first;

            let clients = manager.resolve(None, Some(path));
            let us = undo_store.clone();
            let lim = *limits;
            execute_on_first("edit_range", clients, |client| {
                let p = path.clone();
                let sym = symbol_name.clone();
                let bc = before_ctx.clone();
                let ac = after_ctx.clone();
                let content = new_content.clone();
                let sd = search_dir.clone();
                let us = us.clone();
                async move {
                    edit::execute_edit_range(
                        &client,
                        &p,
                        &sym,
                        bc.as_deref(),
                        ac.as_deref(),
                        &content,
                        sd.as_deref(),
                        &us,
                        &lim,
                    )
                    .await
                }
            })
            .await
        }

        // Process / trigger operations
        Operation::StartProcess { .. }
        | Operation::StopProcess { .. }
        | Operation::SearchProcessOutput { .. }
        | Operation::DefineTrigger { .. }
        | Operation::AwaitTrigger { .. } => process_ops::execute(op, &background).await,

        // Task management operations
        Operation::SetTask { .. }
        | Operation::UpdateTask { .. }
        | Operation::AddSubtask { .. }
        | Operation::CompleteTask { .. }
        | Operation::CompleteSubtask { .. }
        | Operation::ListTasks { .. }
        | Operation::ListSubtasks { .. } => task_ops::execute(op, &background).await,
    }
}
