//! Task management operation execution.
//!
//! Handlers for `SetTask`, `UpdateTask`, `AddSubtask`, `CompleteTask`,
//! `CompleteSubtask`, `ListTasks`, and `ListSubtasks` extracted from `execute_one`.

use std::sync::Arc;

use crate::background::BackgroundManager;

use super::operation::{Operation, OperationResult};

/// Execute a task-management operation.
///
/// # Panics
/// Panics if `op` is not a task management variant.
pub async fn execute(op: Operation, background: &Arc<BackgroundManager>) -> OperationResult {
    match op {
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

        _ => panic!("task_ops::execute called with non-task operation"),
    }
}
