//! File-based DSL operation builders (read, grep).

use std::path::Path;

use crate::tools::Operation;

use super::resolve_file;

/// `read <file> [start end]` or `read` (uses cd_file)
pub fn handle_read(ops: &mut Vec<Operation>, args: &str, cd_dir: &Path, cd_file: Option<&Path>) {
    let parts: Vec<&str> = args.split_whitespace().collect();
    if !parts.is_empty() {
        let file = resolve_file(cd_dir, parts[0]);
        let start_line = parts.get(1).and_then(|s| s.parse().ok()).unwrap_or(0);
        let end_line = parts.get(2).and_then(|s| s.parse().ok()).unwrap_or(0);
        ops.push(Operation::ReadFile {
            file_path: file,
            start_line,
            end_line,
        });
    } else if let Some(f) = cd_file {
        ops.push(Operation::ReadFile {
            file_path: f.display().to_string(),
            start_line: 0,
            end_line: 0,
        });
    }
}

/// `grep <pattern>` — scoped to cd_dir if set
pub fn handle_grep(ops: &mut Vec<Operation>, args: &str, cd_dir: &Path) {
    let trimmed = args.trim();
    if !trimmed.is_empty() {
        let search_dir = if cd_dir.as_os_str().is_empty() {
            None
        } else {
            Some(cd_dir.display().to_string())
        };
        ops.push(Operation::Grep {
            pattern: trimmed.to_string(),
            search_dir,
        });
    }
}
