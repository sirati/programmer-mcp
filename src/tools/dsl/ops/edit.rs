/// DSL parsing for `edit` and `apply_edit` commands.
///
/// Syntax:
///   edit <type_list> <path> <symbol> <new_content>
///   apply_edit <id> <path> <symbol>
///
/// `type_list` is a comma-separated list of: body, signature, docs, file
/// `new_content` is everything after the symbol name.
use std::path::Path;

use super::join_path;
use crate::tools::edit::EditType;
use crate::tools::operation::Operation;

/// Parse `edit <types> <path> <symbol> <content>`.
///
/// Returns `None` and pushes a warning if parsing fails.
pub fn handle_edit(
    ops: &mut Vec<Operation>,
    warnings: &mut Vec<String>,
    args: &str,
    cd_dir: &Path,
    allow_file_edit: bool,
) {
    let args = args.trim();
    if args.is_empty() {
        warnings.push("edit: requires <types> <path> <symbol> <content>".into());
        return;
    }

    // First token: edit types (comma-separated, e.g. "body" or "body,docs")
    let (type_str, rest) = split_first_token(args);
    let edit_types = parse_edit_types(type_str);

    if edit_types.is_empty() {
        warnings.push(format!("edit: unknown edit type(s): {type_str}"));
        return;
    }

    // Check if file edit is allowed
    if edit_types.contains(&EditType::File) && !allow_file_edit {
        warnings.push("edit: file editing is disabled (enable via --allow-file-edit)".into());
        return;
    }

    let rest = rest.trim();
    if rest.is_empty() {
        warnings.push("edit: requires <path> <symbol> <content>".into());
        return;
    }

    // For file-type only edits, syntax is: edit file <path> <content>
    if edit_types.len() == 1 && edit_types[0] == EditType::File {
        let (path_str, content) = split_first_token(rest);
        let path = join_path(cd_dir, path_str);
        ops.push(Operation::Edit {
            edit_types: edit_types.iter().map(|t| t.label().to_string()).collect(),
            path,
            symbol_name: String::new(),
            new_content: unescape_content(content.trim()),
            search_dir: dir_string(cd_dir),
        });
        return;
    }

    // Symbol-based edit: <path> <symbol> <content>
    let (path_str, rest) = split_first_token(rest);
    let path = join_path(cd_dir, path_str);

    let rest = rest.trim();
    if rest.is_empty() {
        warnings.push("edit: requires <symbol> <content>".into());
        return;
    }

    let (symbol, content) = split_first_token(rest);

    if content.trim().is_empty() {
        warnings.push("edit: requires <content> after symbol".into());
        return;
    }

    ops.push(Operation::Edit {
        edit_types: edit_types.iter().map(|t| t.label().to_string()).collect(),
        path,
        symbol_name: symbol.to_string(),
        new_content: unescape_content(content.trim()),
        search_dir: dir_string(cd_dir),
    });
}

/// Parse `apply_edit <id> <path> <symbol>`.
pub fn handle_apply_edit(
    ops: &mut Vec<Operation>,
    warnings: &mut Vec<String>,
    args: &str,
    cd_dir: &Path,
) {
    let args = args.trim();
    if args.is_empty() {
        warnings.push("apply_edit: requires <id> <path> <symbol>".into());
        return;
    }

    let (id, rest) = split_first_token(args);
    let rest = rest.trim();

    if rest.is_empty() {
        warnings.push("apply_edit: requires <path> <symbol>".into());
        return;
    }

    let (path_str, symbol) = split_first_token(rest);
    let path = join_path(cd_dir, path_str);
    let symbol = symbol.trim();

    if symbol.is_empty() {
        warnings.push("apply_edit: requires <symbol>".into());
        return;
    }

    ops.push(Operation::ApplyEdit {
        edit_id: id.to_string(),
        path,
        symbol_name: symbol.to_string(),
    });
}

/// Unescape `\n`, `\t`, `\\` in content strings so multi-line code can be
/// passed on a single DSL line.
fn unescape_content(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        if c == '\\' {
            match chars.next() {
                Some('n') => out.push('\n'),
                Some('t') => out.push('\t'),
                Some('\\') => out.push('\\'),
                Some(other) => {
                    out.push('\\');
                    out.push(other);
                }
                None => out.push('\\'),
            }
        } else {
            out.push(c);
        }
    }
    out
}

/// Parse comma-separated edit types.
fn parse_edit_types(s: &str) -> Vec<EditType> {
    s.split(',')
        .filter_map(|t| EditType::from_str(t.trim()))
        .collect()
}

/// Split into first whitespace-delimited token and the rest.
fn split_first_token(s: &str) -> (&str, &str) {
    let s = s.trim();
    if let Some(pos) = s.find(char::is_whitespace) {
        (&s[..pos], &s[pos..])
    } else {
        (s, "")
    }
}

fn dir_string(cd_dir: &Path) -> Option<String> {
    if cd_dir.as_os_str().is_empty() {
        None
    } else {
        Some(cd_dir.display().to_string())
    }
}
