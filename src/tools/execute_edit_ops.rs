//! Edit operation dispatch: Edit, ApplyEdit, Undo, EditRange.

use crate::config::LengthLimits;
use crate::lsp::manager::LspManager;

use super::edit::{self, PendingEdits, UndoStore};
use super::operation::{Operation, OperationResult};

/// Dispatch an edit-family operation.
pub async fn execute_edit_op(
    manager: &LspManager,
    op: Operation,
    pending_edits: &PendingEdits,
    undo_store: &UndoStore,
    limits: &LengthLimits,
) -> OperationResult {
    match &op {
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

            let types_override: Option<Vec<edit::EditType>> = edit_types.as_ref().map(|ts| {
                ts.iter()
                    .filter_map(|t| edit::EditType::from_str(t))
                    .collect()
            });

            let resolve_path = if let Some(ref p) = path {
                Some(p.clone())
            } else {
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

        _ => OperationResult {
            operation: "unknown".into(),
            success: false,
            output: "not an edit operation".into(),
        },
    }
}
