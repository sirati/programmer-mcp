//! Core operation execution.
//!
//! `execute_batch` runs a list of operations concurrently.
//! `execute_one` dispatches a single `Operation` to the appropriate sub-module.

use std::path::Path;
use std::sync::Arc;

use crate::background::BackgroundManager;
use crate::ipc::HumanMessageBus;
use crate::lsp::manager::LspManager;

use super::execute_lsp;
use super::operation::{Operation, OperationResult};
use super::{process_ops, task_ops, workspace};

/// Execute a batch of operations concurrently.
pub async fn execute_batch(
    manager: &Arc<LspManager>,
    message_bus: &Arc<HumanMessageBus>,
    background: &Arc<BackgroundManager>,
    workspace_root: &Path,
    operations: Vec<Operation>,
) -> Vec<OperationResult> {
    let futures: Vec<_> = operations
        .into_iter()
        .map(|op| {
            let manager = manager.clone();
            let bus = message_bus.clone();
            let bg = background.clone();
            let ws = workspace_root.to_path_buf();
            tokio::spawn(async move { execute_one(&manager, &bus, bg, &ws, op).await })
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
) -> OperationResult {
    match &op {
        // LSP: symbol-based
        Operation::Definition { .. }
        | Operation::References { .. }
        | Operation::Docstring { .. }
        | Operation::Body { .. }
        | Operation::Impls { .. } => execute_lsp::execute_symbol_op(manager, op).await,

        // LSP: file-based
        Operation::Diagnostics { .. }
        | Operation::Hover { .. }
        | Operation::RenameSymbol { .. }
        | Operation::ListSymbols { .. }
        | Operation::CodeActions { .. }
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

        // Workspace info
        Operation::WorkspaceInfo => {
            let output = workspace::workspace_info(workspace_root);
            OperationResult {
                operation: "workspace_info".into(),
                success: true,
                output,
            }
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
