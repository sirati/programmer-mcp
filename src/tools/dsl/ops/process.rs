//! Background-process and trigger DSL operation builders.

use crate::tools::Operation;

use super::super::parse::split_first_word;

/// `start_process <name> <command> [args...] [group=<g>]`
pub fn handle_start_process(ops: &mut Vec<Operation>, args: &str) {
    let (name, rest) = split_first_word(args);
    if name.is_empty() {
        return;
    }
    let (command, rest) = split_first_word(rest);
    if command.is_empty() {
        return;
    }

    let mut group: Option<String> = None;
    let mut process_args: Vec<String> = Vec::new();
    for token in rest.split_whitespace() {
        if let Some(g) = token.strip_prefix("group=") {
            group = Some(g.to_string());
        } else {
            process_args.push(token.to_string());
        }
    }
    ops.push(Operation::StartProcess {
        name: name.to_string(),
        command: command.to_string(),
        args: process_args,
        group,
    });
}

/// `stop_process <name>`
pub fn handle_stop_process(ops: &mut Vec<Operation>, args: &str) {
    let name = args.trim();
    if name.is_empty() {
        return;
    }
    ops.push(Operation::StopProcess {
        name: name.to_string(),
    });
}

/// `search_output <name_or_group> <pattern>`
pub fn handle_search_output(ops: &mut Vec<Operation>, args: &str) {
    let (name, pattern) = split_first_word(args);
    if name.is_empty() || pattern.is_empty() {
        return;
    }
    ops.push(Operation::SearchProcessOutput {
        name: Some(name.to_string()),
        group: None,
        pattern: pattern.to_string(),
    });
}

/// `define_trigger <name> <pattern> [before=N] [after=N] [timeout=N] [group=g]`
pub fn handle_define_trigger(ops: &mut Vec<Operation>, args: &str) {
    let (name, rest) = split_first_word(args);
    if name.is_empty() {
        return;
    }
    let (pattern, opts) = split_first_word(rest);
    if pattern.is_empty() {
        return;
    }

    let mut lines_before = 3usize;
    let mut lines_after = 5usize;
    let mut timeout_ms = 30_000u64;
    let mut group: Option<String> = None;
    for token in opts.split_whitespace() {
        if let Some(v) = token.strip_prefix("before=") {
            lines_before = v.parse().unwrap_or(lines_before);
        } else if let Some(v) = token.strip_prefix("after=") {
            lines_after = v.parse().unwrap_or(lines_after);
        } else if let Some(v) = token.strip_prefix("timeout=") {
            timeout_ms = v.parse().unwrap_or(timeout_ms);
        } else if let Some(g) = token.strip_prefix("group=") {
            group = Some(g.to_string());
        }
    }
    ops.push(Operation::DefineTrigger {
        name: name.to_string(),
        pattern: pattern.to_string(),
        lines_before,
        lines_after,
        timeout_ms,
        group,
    });
}

/// `await_trigger <name>`
pub fn handle_await_trigger(ops: &mut Vec<Operation>, args: &str) {
    let name = args.trim();
    if name.is_empty() {
        return;
    }
    ops.push(Operation::AwaitTrigger {
        name: name.to_string(),
    });
}
