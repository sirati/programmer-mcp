//! LSP-related DSL operation builders.

use std::path::Path;

use crate::lsp::detect_lang::detect_language_id;
use crate::tools::Operation;
use crate::tools::SOURCE_EXTS;

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
        } else {
            // No file context — list directory contents
            ops.push(Operation::ListDir {
                dir_path: cd_dir.display().to_string(),
                max_depth: 1,
            });
        }
        return;
    }
    for item in parse_item_list(args) {
        if let Some(path) = resolve_list_item(&item, cd_dir, cd_file) {
            // Check if the resolved path is a directory
            let abs = if Path::new(&path).is_absolute() {
                std::path::PathBuf::from(&path)
            } else if let Ok(cwd) = std::env::current_dir() {
                cwd.join(&path)
            } else {
                std::path::PathBuf::from(&path)
            };
            if abs.is_dir() {
                ops.push(Operation::ListDir {
                    dir_path: path,
                    max_depth: 1,
                });
            } else {
                ops.push(Operation::ListSymbols {
                    file_path: path,
                    max_depth: 3,
                    language: None,
                });
            }
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

/// Dispatch a symbol-based command, auto-correcting path-first usage.
///
/// When bare args like `body src/foo.rs my_fn` are used (path first, no brackets),
/// detects the path and auto-corrects to `body [src/foo.rs my_fn]`.
pub fn handle_symbol_cmd(
    ops: &mut Vec<Operation>,
    warnings: &mut Vec<String>,
    cmd: &str,
    args: &str,
    cd_dir: &Path,
    cd_file: Option<&Path>,
) {
    let trimmed = args.trim();
    if trimmed.is_empty() {
        return;
    }

    // Detect path-first bare args: `body src/foo.rs my_fn`
    // Auto-correct to bracket form: `body [src/foo.rs my_fn]`
    if !trimmed.starts_with('[') {
        let items: Vec<&str> = trimmed.split_whitespace().collect();
        if items.len() > 1 && looks_like_path(items[0]) {
            let sym_list = items[1..].join(", ");
            warnings.push(format!(
                "incorrect arguments, corrected to: `{cmd} {}.{{{sym_list}, .}}`",
                items[0],
            ));
            // Re-parse as bracket form — path becomes search context
            let bracket_args = format!("[{trimmed}]");
            let syms = non_empty_items(&bracket_args);
            if syms.is_empty() {
                return;
            }
            // The first item resolved as a path provides search_dir
            let resolved_path = resolve_file(cd_dir, items[0]);
            let path = Path::new(&resolved_path);
            let dir = if path.extension().is_some() {
                path.parent().map(|p| p.display().to_string())
            } else {
                Some(resolved_path.clone())
            };
            let search_dir = dir.or_else(|| {
                if cd_dir.as_os_str().is_empty() {
                    None
                } else {
                    Some(cd_dir.display().to_string())
                }
            });
            // Detect language from file extension to scope the search.
            let lang = detect_language_id(items[0]);
            let language = if lang.is_empty() {
                None
            } else {
                Some(lang.to_string())
            };
            // Symbol names are everything after the path
            let symbol_names: Vec<String> = items[1..].iter().map(|s| s.to_string()).collect();
            push_symbol_op(ops, cmd, symbol_names, language, search_dir);
            return;
        }
        if items.len() > 1 {
            warnings.push(format!(
                "command `{cmd} {trimmed}` was used without brackets — \
                 correct usage: `{cmd} [{}]`",
                items.join(" ")
            ));
        }
    }

    let syms = non_empty_items(args);
    if syms.is_empty() {
        return;
    }
    let search_dir = if cd_dir.as_os_str().is_empty() {
        // When cd_file is set at workspace root, use "." so directory walk still works.
        if cd_file.is_some() {
            Some(".".to_string())
        } else {
            None
        }
    } else {
        Some(cd_dir.display().to_string())
    };
    // When cd_file is set, scope search to that file's language.
    let language = cd_file.and_then(|f| {
        let lang = detect_language_id(&f.display().to_string());
        if lang.is_empty() {
            None
        } else {
            Some(lang.to_string())
        }
    });
    push_symbol_op(ops, cmd, syms, language, search_dir);
}

/// Check if a string looks like a file path (contains `/` or has a source extension).
fn looks_like_path(s: &str) -> bool {
    if s.contains('/') {
        return true;
    }
    if let Some(ext) = Path::new(s).extension().and_then(|e| e.to_str()) {
        return SOURCE_EXTS.contains(&ext);
    }
    false
}

fn push_symbol_op(
    ops: &mut Vec<Operation>,
    cmd: &str,
    symbol_names: Vec<String>,
    language: Option<String>,
    search_dir: Option<String>,
) {
    let op = match cmd {
        "body" => Operation::Body {
            symbol_names,
            language,
            search_dir,
        },
        "definition" => Operation::Definition {
            symbol_names,
            language,
            search_dir,
        },
        "references" => Operation::References {
            symbol_names,
            language,
            search_dir,
        },
        "docstring" => Operation::Docstring {
            symbol_names,
            language,
            search_dir,
        },
        "impls" => Operation::Impls {
            symbol_names,
            language,
            search_dir,
        },
        "callers" => Operation::Callers {
            symbol_names,
            language,
            search_dir,
        },
        "callees" => Operation::Callees {
            symbol_names,
            language,
            search_dir,
        },
        _ => return,
    };
    ops.push(op);
}
