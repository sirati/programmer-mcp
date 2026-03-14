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

use super::edit::{PendingEdits, UndoStore};
use super::execute_edit_ops;
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
        | Operation::Impls { .. }
        | Operation::HoverSymbol { .. }
        | Operation::RenameBySymbol { .. } => execute_lsp::execute_symbol_op(manager, op).await,

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
            let output =
                grep::grep_workspace(pattern, search_dir.as_deref(), workspace_root, manager).await;
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
            let output = workspace::workspace_info(workspace_root, manager).await;
            OperationResult {
                operation: "workspace_info".into(),
                success: true,
                output,
            }
        }

        // Symbol search
        Operation::SearchSymbols {
            query,
            language,
            limit,
        } => {
            use super::symbol_search_cmd::execute_search_symbols;
            execute_search_symbols(manager, &query, language.as_deref(), *limit).await
        }

        // Edit operations (need LSP + pending edits)
        Operation::Edit { .. }
        | Operation::ApplyEdit { .. }
        | Operation::Undo { .. }
        | Operation::EditRange { .. } => {
            execute_edit_ops::execute_edit_op(manager, op, pending_edits, undo_store, limits).await
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
