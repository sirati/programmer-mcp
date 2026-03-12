//! Read file contents with optional line range.

use std::fmt::Write;
use std::path::Path;

/// Read file contents, optionally restricted to a line range.
///
/// If `start_line` and `end_line` are both 0, reads the whole file (up to a limit).
/// Lines are 1-indexed.
pub fn read_file(
    file_path: &str,
    start_line: usize,
    end_line: usize,
    workspace_root: &Path,
) -> String {
    let abs = if Path::new(file_path).is_absolute() {
        file_path.into()
    } else {
        workspace_root.join(file_path).display().to_string()
    };

    let content = match std::fs::read_to_string(&abs) {
        Ok(c) => c,
        Err(e) => return format!("{file_path}: {e}"),
    };

    let lines: Vec<&str> = content.lines().collect();
    let total = lines.len();

    let (start, end) = if start_line == 0 && end_line == 0 {
        // Read whole file, capped at 200 lines
        (0, total.min(200))
    } else {
        let s = start_line.saturating_sub(1).min(total);
        let e = if end_line == 0 {
            (s + 50).min(total) // Default: 50 lines from start
        } else {
            end_line.min(total)
        };
        (s, e)
    };

    let mut out = String::new();
    let width = format!("{}", end).len();

    for (i, line) in lines[start..end].iter().enumerate() {
        let line_num = start + i + 1;
        writeln!(out, "{line_num:>width$}| {line}").ok();
    }

    if end < total {
        writeln!(out, "... ({} more lines, {} total)", total - end, total).ok();
    }

    out.trim_end().to_string()
}
