//! Refactoring-related DSL operation builders (code actions, format).

use std::path::Path;

use crate::tools::Operation;

use super::super::parse::parse_item_list;
use super::{resolve_file, resolve_list_item};

/// `code_action <file> <line> <col> [end_line end_col] [kind1 kind2 ...]`
/// or `code_action <line> <col>` (uses cd_file)
pub fn handle_code_action(
    ops: &mut Vec<Operation>,
    args: &str,
    cd_dir: &Path,
    cd_file: Option<&Path>,
) {
    let parts: Vec<&str> = args.split_whitespace().collect();
    let (file, rest) = if parts.len() >= 3 {
        // Try to parse first arg as a number — if it is, use cd_file
        if parts[0].parse::<u32>().is_ok() {
            let Some(f) = cd_file else { return };
            (f.display().to_string(), &parts[..])
        } else {
            (resolve_file(cd_dir, parts[0]), &parts[1..])
        }
    } else if parts.len() == 2 {
        let Some(f) = cd_file else { return };
        (f.display().to_string(), &parts[..])
    } else {
        return;
    };
    if rest.len() < 2 {
        return;
    }
    let Ok(line) = rest[0].parse::<u32>() else {
        return;
    };
    let Ok(column) = rest[1].parse::<u32>() else {
        return;
    };

    // Optional end_line end_col and kinds
    let mut end_line = None;
    let mut end_column = None;
    let mut kinds = Vec::new();
    let mut i = 2;
    if i + 1 < rest.len() {
        if let (Ok(el), Ok(ec)) = (rest[i].parse::<u32>(), rest[i + 1].parse::<u32>()) {
            end_line = Some(el);
            end_column = Some(ec);
            i += 2;
        }
    }
    for &k in &rest[i..] {
        kinds.push(k.to_string());
    }

    ops.push(Operation::CodeAction {
        file_path: file,
        line,
        column,
        end_line,
        end_column,
        kinds,
        language: None,
    });
}

/// `code_actions <file> <line> <col>` or `code_actions <line> <col>` (uses cd_file)
pub fn handle_code_actions(
    ops: &mut Vec<Operation>,
    args: &str,
    cd_dir: &Path,
    cd_file: Option<&Path>,
) {
    let (file, line, column) = match parse_file_line_col(args, cd_dir, cd_file) {
        Some(v) => v,
        None => return,
    };
    ops.push(Operation::CodeActions {
        file_path: file,
        line,
        column,
        language: None,
    });
}

/// `apply_action <file> <line> <col> <index>` or `apply_action <line> <col> <index>`
pub fn handle_apply_action(
    ops: &mut Vec<Operation>,
    args: &str,
    cd_dir: &Path,
    cd_file: Option<&Path>,
) {
    let parts: Vec<&str> = args.split_whitespace().collect();
    let (file, line_str, col_str, idx_str) = if parts.len() >= 4 {
        (resolve_file(cd_dir, parts[0]), parts[1], parts[2], parts[3])
    } else if parts.len() == 3 {
        let Some(f) = cd_file else { return };
        (f.display().to_string(), parts[0], parts[1], parts[2])
    } else {
        return;
    };
    let Ok(line) = line_str.parse::<u32>() else {
        return;
    };
    let Ok(column) = col_str.parse::<u32>() else {
        return;
    };
    let Ok(index) = idx_str.parse::<usize>() else {
        return;
    };
    ops.push(Operation::ApplyCodeAction {
        file_path: file,
        line,
        column,
        index,
        language: None,
    });
}

/// `format [files]` or bare `format` (uses cd_file)
pub fn handle_format(ops: &mut Vec<Operation>, args: &str, cd_dir: &Path, cd_file: Option<&Path>) {
    if args.trim().is_empty() {
        if let Some(f) = cd_file {
            ops.push(Operation::Format {
                file_path: f.display().to_string(),
                language: None,
            });
        }
        return;
    }
    for item in parse_item_list(args) {
        if let Some(path) = resolve_list_item(&item, cd_dir, cd_file) {
            ops.push(Operation::Format {
                file_path: path,
                language: None,
            });
        }
    }
}

/// Shared helper: parse `<file> <line> <col>` or `<line> <col>` from args.
fn parse_file_line_col(
    args: &str,
    cd_dir: &Path,
    cd_file: Option<&Path>,
) -> Option<(String, u32, u32)> {
    let parts: Vec<&str> = args.split_whitespace().collect();
    let (file, line_str, col_str) = if parts.len() >= 3 {
        (resolve_file(cd_dir, parts[0]), parts[1], parts[2])
    } else if parts.len() == 2 {
        let f = cd_file?;
        (f.display().to_string(), parts[0], parts[1])
    } else {
        return None;
    };
    let line = line_str.parse::<u32>().ok()?;
    let column = col_str.parse::<u32>().ok()?;
    Some((file, line, column))
}
