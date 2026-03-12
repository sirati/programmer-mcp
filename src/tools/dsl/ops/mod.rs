//! DSL command → Operation builders.
//!
//! Each function handles one DSL command keyword and produces zero or more
//! `Operation` values appended to the provided `ops` Vec.

use std::path::Path;

pub mod lsp;
pub mod process;
pub mod task;

pub use lsp::{
    handle_apply_action, handle_body, handle_code_actions, handle_definition, handle_diagnostics,
    handle_docstring, handle_format, handle_hover, handle_impls, handle_list_symbols,
    handle_references, handle_rename_symbol,
};
pub use process::{
    handle_await_trigger, handle_define_trigger, handle_search_output, handle_start_process,
    handle_stop_process,
};
pub use task::{
    handle_add_subtask, handle_complete_subtask, handle_complete_task, handle_list_subtasks,
    handle_list_tasks, handle_set_task, handle_update_task,
};

// ── path helpers ──────────────────────────────────────────────────────────────

/// Recognized source-file extensions used for optional-extension resolution.
pub const SOURCE_EXTENSIONS: &[&str] = &[
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

// ── shared list helpers ───────────────────────────────────────────────────────

/// Resolve a file item from a list, handling the special `.` token (current cd file).
pub fn resolve_list_item(item: &str, cd_dir: &Path, cd_file: Option<&Path>) -> Option<String> {
    if item == "." {
        cd_file.map(|f| f.display().to_string())
    } else {
        Some(resolve_file(cd_dir, item))
    }
}

/// Filter `.` (current-file sentinel) from symbol name lists.
pub fn non_empty_items(args: &str) -> Vec<String> {
    use super::parse::parse_item_list;
    parse_item_list(args)
        .into_iter()
        .filter(|s| s != ".")
        .collect()
}
