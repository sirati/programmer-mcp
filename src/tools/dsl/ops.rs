//! DSL command → Operation builders.
//!
//! Each function handles one DSL command keyword and produces zero or more
//! `Operation` values appended to the provided `ops` Vec.

use std::path::Path;

use crate::tools::Operation;

use super::parse::{parse_item_list, split_first_word};

// ── path helpers ──────────────────────────────────────────────────────────────

/// Recognized source-file extensions used for optional-extension resolution.
const SOURCE_EXTENSIONS: &[&str] = &[
    "rs", "py", "go", "ts", "tsx", "js", "jsx", "c", "cpp", "cc", "h", "hpp", "java", "kt", "rb",
    "cs", "swift", "zig", "lua", "ml", "mli",
];

/// Normalize `..` and `.` components without hitting the filesystem.
pub fn normalize_path(path: &Path) -> std::path::PathBuf {
    let mut out = std::path::PathBuf::new();
    for c in path.components() {
        match c {
            std::path::Component::ParentDir => {
                out.pop();
            }
            std::path::Component::CurDir => {}
            other => out.push(other),
        }
    }
    out
}

/// Join `base_dir` with `item`, normalising the result.
/// If `item` is absolute it is used as-is.
pub fn join_path(base_dir: &Path, item: &str) -> String {
    let p = if Path::new(item).is_absolute() {
        Path::new(item).to_path_buf()
    } else if base_dir.as_os_str().is_empty() {
        Path::new(item).to_path_buf()
    } else {
        base_dir.join(item)
    };
    normalize_path(&p).display().to_string()
}

/// Join `base_dir` with `item` and, if the result has no extension, attempt to
/// locate the file by probing common source extensions via the current working
/// directory (which equals the workspace root after server start-up).
pub fn resolve_file(base_dir: &Path, item: &str) -> String {
    let joined = join_path(base_dir, item);
    let rel = Path::new(&joined);
    if rel.extension().is_some() {
        return joined;
    }
    if let Ok(cwd) = std::env::current_dir() {
        for ext in SOURCE_EXTENSIONS {
            let candidate = cwd.join(rel).with_extension(ext);
            if candidate.exists() {
                return rel.with_extension(ext).display().to_string();
            }
        }
    }
    joined
}

/// Return `true` if `path` has a file extension (used to distinguish files from dirs in `cd`).
pub fn has_extension(path: &Path) -> bool {
    path.extension().is_some()
}

/// Attempt to resolve a bare path (no extension) by probing the workspace.
/// Used by `cd` so that `cd src/main` finds `src/main.rs` etc.
pub fn resolve_cd_path(path: &Path) -> std::path::PathBuf {
    if path.extension().is_some() {
        return path.to_path_buf();
    }
    if let Ok(cwd) = std::env::current_dir() {
        for ext in SOURCE_EXTENSIONS {
            let candidate = cwd.join(path).with_extension(ext);
            if candidate.exists() {
                return path.with_extension(ext);
            }
        }
    }
    path.to_path_buf()
}

// ── file-based operations ─────────────────────────────────────────────────────

/// Resolve a file item from a list, handling the special `.` token (current cd file).
fn resolve_list_item(item: &str, cd_dir: &Path, cd_file: Option<&Path>) -> Option<String> {
    if item == "." {
        cd_file.map(|f| f.display().to_string())
    } else {
        Some(resolve_file(cd_dir, item))
    }
}

pub fn handle_list_symbols(
    ops: &mut Vec<Operation>,
    args: &str,
    cd_dir: &Path,
    cd_file: Option<&Path>,
) {
    if args.trim().is_empty() {
        if let Some(f) = cd_file {
            ops.push(Operation::ListSymbols {
                file_path: f.display().to_string(),
                max_depth: 3,
                language: None,
            });
        }
        return;
    }
    for item in parse_item_list(args) {
        if let Some(path) = resolve_list_item(&item, cd_dir, cd_file) {
            ops.push(Operation::ListSymbols {
                file_path: path,
                max_depth: 3,
                language: None,
            });
        }
    }
}

pub fn handle_diagnostics(
    ops: &mut Vec<Operation>,
    args: &str,
    cd_dir: &Path,
    cd_file: Option<&Path>,
) {
    for item in parse_item_list(args) {
        if let Some(path) = resolve_list_item(&item, cd_dir, cd_file) {
            ops.push(Operation::Diagnostics {
                file_path: path,
                context_lines: 5,
                show_line_numbers: true,
                language: None,
            });
        }
    }
}

/// `hover <file> <line> <col>` or `hover <line> <col>` (uses cd_file)
pub fn handle_hover(ops: &mut Vec<Operation>, args: &str, cd_dir: &Path, cd_file: Option<&Path>) {
    let parts: Vec<&str> = args.split_whitespace().collect();
    let (file, line_str, col_str) = if parts.len() >= 3 {
        (resolve_file(cd_dir, parts[0]), parts[1], parts[2])
    } else if parts.len() == 2 {
        let Some(f) = cd_file else { return };
        (f.display().to_string(), parts[0], parts[1])
    } else {
        return;
    };
    let Ok(line) = line_str.parse::<u32>() else {
        return;
    };
    let Ok(column) = col_str.parse::<u32>() else {
        return;
    };
    ops.push(Operation::Hover {
        file_path: file,
        line,
        column,
        language: None,
    });
}

/// `rename_symbol <file> <line> <col> <new_name>` or `rename_symbol <line> <col> <new_name>`
pub fn handle_rename_symbol(
    ops: &mut Vec<Operation>,
    args: &str,
    cd_dir: &Path,
    cd_file: Option<&Path>,
) {
    let parts: Vec<&str> = args.splitn(4, char::is_whitespace).collect();
    let (file, line_str, col_str, new_name) = if parts.len() == 4 {
        (resolve_file(cd_dir, parts[0]), parts[1], parts[2], parts[3])
    } else if parts.len() == 3 {
        let Some(f) = cd_file else { return };
        (f.display().to_string(), parts[0], parts[1], parts[2])
    } else {
        return;
    };
    let Ok(line) = line_str.trim().parse::<u32>() else {
        return;
    };
    let Ok(column) = col_str.trim().parse::<u32>() else {
        return;
    };
    ops.push(Operation::RenameSymbol {
        file_path: file,
        line,
        column,
        new_name: new_name.trim().to_string(),
        language: None,
    });
}

// ── symbol-based operations ───────────────────────────────────────────────────

pub fn handle_body(ops: &mut Vec<Operation>, args: &str) {
    let syms = non_empty_items(args);
    if !syms.is_empty() {
        ops.push(Operation::Body {
            symbol_names: syms,
            language: None,
        });
    }
}

pub fn handle_definition(ops: &mut Vec<Operation>, args: &str) {
    let syms = non_empty_items(args);
    if !syms.is_empty() {
        ops.push(Operation::Definition {
            symbol_names: syms,
            language: None,
        });
    }
}

pub fn handle_references(ops: &mut Vec<Operation>, args: &str) {
    let syms = non_empty_items(args);
    if !syms.is_empty() {
        ops.push(Operation::References {
            symbol_names: syms,
            language: None,
        });
    }
}

pub fn handle_docstring(ops: &mut Vec<Operation>, args: &str) {
    let syms = non_empty_items(args);
    if !syms.is_empty() {
        ops.push(Operation::Docstring {
            symbol_names: syms,
            language: None,
        });
    }
}

pub fn handle_impls(ops: &mut Vec<Operation>, args: &str) {
    let syms = non_empty_items(args);
    if !syms.is_empty() {
        ops.push(Operation::Impls {
            symbol_names: syms,
            language: None,
        });
    }
}

/// Filter `.` (current-file sentinel) from symbol name lists.
fn non_empty_items(args: &str) -> Vec<String> {
    parse_item_list(args)
        .into_iter()
        .filter(|s| s != ".")
        .collect()
}

// ── task operations ───────────────────────────────────────────────────────────

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

// ── background process / trigger operations ───────────────────────────────────

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
