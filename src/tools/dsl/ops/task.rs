//! Task-management DSL operation builders.

use crate::tools::Operation;

use super::super::parse::split_first_word;

/// `set_task <name> <description>`
pub fn handle_set_task(ops: &mut Vec<Operation>, args: &str) {
    let (name, desc) = split_first_word(args);
    if name.is_empty() {
        return;
    }
    ops.push(Operation::SetTask {
        name: name.to_string(),
        description: desc.to_string(),
    });
}

/// `update_task <name> <new_description>` or `update_task <name> append=<text>`
pub fn handle_update_task(ops: &mut Vec<Operation>, args: &str) {
    let (name, rest) = split_first_word(args);
    if name.is_empty() {
        return;
    }
    let (key, val) = split_first_word(rest);
    let (new_desc, append_desc) = if key.starts_with("append=") {
        (
            None,
            Some(key.trim_start_matches("append=").to_string() + " " + val),
        )
    } else {
        (Some(rest.to_string()), None)
    };
    ops.push(Operation::UpdateTask {
        name: name.to_string(),
        new_description: new_desc.filter(|s| !s.is_empty()),
        append_description: append_desc.filter(|s| !s.trim().is_empty()),
        completed: None,
    });
}

/// `complete_task <name>`
pub fn handle_complete_task(ops: &mut Vec<Operation>, args: &str) {
    let name = args.trim();
    if name.is_empty() {
        return;
    }
    ops.push(Operation::CompleteTask {
        name: name.to_string(),
    });
}

/// `list_tasks [completed]`
pub fn handle_list_tasks(ops: &mut Vec<Operation>, args: &str) {
    ops.push(Operation::ListTasks {
        include_completed: args.contains("completed"),
    });
}

/// `add_subtask <task> <sub> <description>`
pub fn handle_add_subtask(ops: &mut Vec<Operation>, args: &str) {
    let (task, rest) = split_first_word(args);
    let (sub, desc) = split_first_word(rest);
    if task.is_empty() || sub.is_empty() {
        return;
    }
    ops.push(Operation::AddSubtask {
        task_name: task.to_string(),
        subtask_name: sub.to_string(),
        description: desc.to_string(),
    });
}

/// `complete_subtask <task> <sub>`
pub fn handle_complete_subtask(ops: &mut Vec<Operation>, args: &str) {
    let (task, rest) = split_first_word(args);
    let (sub, _) = split_first_word(rest);
    if task.is_empty() || sub.is_empty() {
        return;
    }
    ops.push(Operation::CompleteSubtask {
        task_name: task.to_string(),
        subtask_name: sub.to_string(),
    });
}

/// `list_subtasks <task> [completed]`
pub fn handle_list_subtasks(ops: &mut Vec<Operation>, args: &str) {
    let (task, rest) = split_first_word(args);
    if task.is_empty() {
        return;
    }
    ops.push(Operation::ListSubtasks {
        task_name: task.to_string(),
        include_completed: rest.contains("completed"),
    });
}
