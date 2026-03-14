//! File-based DSL operation builders (read, grep, search, list_symbols, diagnostics).

use std::path::Path;

use crate::tools::dsl::parse::{parse_item_list, unquote};
use crate::tools::Operation;

use super::{detect_dir_language, resolve_file, resolve_list_item};

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

/// `search <query> [limit=N]` — fuzzy symbol search across the index
pub fn handle_search_symbols(
    ops: &mut Vec<Operation>,
    args: &str,
    cd_dir: &Path,
    cd_file: Option<&Path>,
) {
    let parts: Vec<&str> = args.split_whitespace().collect();
    if parts.is_empty() {
        return;
    }

    let mut query_parts = Vec::new();
    let mut limit = 20usize;

    for part in &parts {
        if let Some(n) = part.strip_prefix("limit=") {
            if let Ok(l) = n.parse::<usize>() {
                limit = l;
            }
        } else {
            query_parts.push(*part);
        }
    }

    let query = unquote(&query_parts.join(" "));
    if query.is_empty() {
        return;
    }

    let language = cd_file
        .and_then(|f| {
            let lang = crate::lsp::detect_lang::detect_language_id(&f.display().to_string());
            if lang.is_empty() {
                None
            } else {
                Some(lang.to_string())
            }
        })
        .or_else(|| detect_dir_language(cd_dir));

    ops.push(Operation::SearchSymbols {
        query,
        language,
        limit,
    });
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
            pattern: unquote(trimmed),
            search_dir,
        });
    }
}

/// `list_symbols [f1 f2]` — symbol tree (on dirs: list source files)
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
            ops.push(Operation::ListDir {
                dir_path: cd_dir.display().to_string(),
                max_depth: 1,
            });
        }
        return;
    }
    for item in parse_item_list(args) {
        if let Some(path) = resolve_list_item(&item, cd_dir, cd_file) {
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

/// `diagnostics [f1 f2]` — errors/warnings for files
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
