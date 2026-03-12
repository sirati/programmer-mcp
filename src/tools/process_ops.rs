//! Process and trigger operation execution.
//!
//! Handlers for `StartProcess`, `StopProcess`, `SearchProcessOutput`,
//! `DefineTrigger`, and `AwaitTrigger` extracted from `execute_one`.

use std::sync::Arc;

use crate::background::BackgroundManager;

use super::operation::{Operation, OperationResult};

/// Execute a process- or trigger-related operation.
///
/// # Panics
/// Panics if `op` is not a process or trigger variant.
pub async fn execute(op: Operation, background: &Arc<BackgroundManager>) -> OperationResult {
    match op {
        Operation::StartProcess {
            name,
            command,
            args,
            group,
        } => execute_start_process(name, command, args, group, background).await,

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

        Operation::AwaitTrigger { name } => execute_await_trigger(name, background).await,

        _ => panic!("process_ops::execute called with non-process operation"),
    }
}

async fn execute_start_process(
    name: String,
    command: String,
    args: Vec<String>,
    group: Option<String>,
    background: &Arc<BackgroundManager>,
) -> OperationResult {
    let result =
        background
            .processes
            .lock()
            .await
            .start(name.clone(), group.clone(), &command, &args);

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

async fn execute_await_trigger(
    name: String,
    background: &Arc<BackgroundManager>,
) -> OperationResult {
    // Check if already fired
    {
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
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_millis(timeout_ms);

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

/// Spawn a task that watches a process's output and fires a trigger when the pattern matches.
pub async fn run_trigger_scanner(
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
                        _ => break,
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
