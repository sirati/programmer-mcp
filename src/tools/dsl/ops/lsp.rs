//! LSP-related DSL operation builders.

use std::path::Path;

use crate::lsp::detect_lang::{detect_dir_language, detect_language_id};
use crate::tools::Operation;
use crate::tools::SOURCE_EXTS;

use super::{non_empty_items, resolve_file};

/// Resolve `language` and `search_dir` from DSL context (cd_dir / cd_file).
fn resolve_lang_and_dir(cd_dir: &Path, cd_file: Option<&Path>) -> (Option<String>, Option<String>) {
    let search_dir = if cd_dir.as_os_str().is_empty() {
        cd_file.map(|_| ".".to_string())
    } else {
        Some(cd_dir.display().to_string())
    };
    let language = cd_file
        .and_then(|f| {
            let lang = detect_language_id(&f.display().to_string());
            if lang.is_empty() {
                None
            } else {
                Some(lang.to_string())
            }
        })
        .or_else(|| detect_dir_language(cd_dir));
    (language, search_dir)
}

/// `hover <file> <line> <col>` or `hover <line> <col>` (uses cd_file)
/// Also supports `hover [symbol_name]` to auto-resolve position via symbol search.
pub fn handle_hover(ops: &mut Vec<Operation>, args: &str, cd_dir: &Path, cd_file: Option<&Path>) {
    let parts: Vec<&str> = args.split_whitespace().collect();

    // Try positional form first: `hover file line col` or `hover line col`
    if parts.len() >= 2 {
        // Check if the last two args are numbers (line col)
        let last_is_num = parts[parts.len() - 1].parse::<u32>().is_ok();
        let second_last_is_num = parts[parts.len() - 2].parse::<u32>().is_ok();

        if last_is_num && second_last_is_num {
            let (file, line_str, col_str) = if parts.len() >= 3 {
                (
                    resolve_file(cd_dir, parts[0]),
                    parts[parts.len() - 2],
                    parts[parts.len() - 1],
                )
            } else {
                let Some(f) = cd_file else { return };
                (f.display().to_string(), parts[0], parts[1])
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
            return;
        }
    }

    // Symbol-name form: `hover [sym1 sym2]` or `hover sym_name`
    let syms = non_empty_items(args);
    if syms.is_empty() {
        return;
    }
    let (language, search_dir) = resolve_lang_and_dir(cd_dir, cd_file);
    ops.push(Operation::HoverSymbol {
        symbol_names: syms,
        language,
        search_dir,
    });
}

/// `rename_symbol <file> <line> <col> <new_name>` or `rename_symbol <line> <col> <new_name>`
/// Also supports `rename_symbol <symbol_name> <new_name>` to auto-resolve position.
pub fn handle_rename_symbol(
    ops: &mut Vec<Operation>,
    args: &str,
    cd_dir: &Path,
    cd_file: Option<&Path>,
) {
    let parts: Vec<&str> = args.splitn(4, char::is_whitespace).collect();

    // Try positional form: `rename_symbol file line col new_name` or `rename_symbol line col new_name`
    if parts.len() >= 3 {
        let maybe_line = parts[parts.len() - 3].parse::<u32>();
        let maybe_col = parts[parts.len() - 2].parse::<u32>();
        if let (Ok(line), Ok(column)) = (maybe_line, maybe_col) {
            let new_name = parts[parts.len() - 1];
            let file = if parts.len() == 4 {
                resolve_file(cd_dir, parts[0])
            } else if let Some(f) = cd_file {
                f.display().to_string()
            } else {
                return;
            };
            ops.push(Operation::RenameSymbol {
                file_path: file,
                line,
                column,
                new_name: new_name.trim().to_string(),
                language: None,
            });
            return;
        }
    }

    // Symbol-name form: `rename_symbol <symbol_name> <new_name>`
    if parts.len() == 2 {
        let (language, search_dir) = resolve_lang_and_dir(cd_dir, cd_file);
        ops.push(Operation::RenameBySymbol {
            symbol_name: parts[0].to_string(),
            new_name: parts[1].trim().to_string(),
            language,
            search_dir,
        });
        return;
    }

    // Not enough args
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
    let (language, search_dir) = resolve_lang_and_dir(cd_dir, cd_file);
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
    macro_rules! sym_op {
        ($variant:ident) => {
            Operation::$variant {
                symbol_names,
                language,
                search_dir,
            }
        };
    }
    let op = match cmd {
        "body" => sym_op!(Body),
        "definition" => sym_op!(Definition),
        "references" => sym_op!(References),
        "docstring" => sym_op!(Docstring),
        "impls" => sym_op!(Impls),
        "callers" => sym_op!(Callers),
        "callees" => sym_op!(Callees),
        _ => return,
    };
    ops.push(op);
}
