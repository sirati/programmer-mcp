//! LSP-related DSL operation builders.

use std::path::Path;

use crate::tools::Operation;

use super::super::parse::parse_item_list;
use super::{non_empty_items, resolve_file, resolve_list_item};

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
