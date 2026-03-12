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

/// Parse `apply_edit` in three forms:
///
/// 1. `apply_edit <id>` — confirm with stored args
/// 2. `apply_edit <id> [type1 type2 ...]` — override edit types, keep stored path/symbol
/// 3. `apply_edit <id> <path> <symbol>` — correct location
pub fn handle_apply_edit(
    ops: &mut Vec<Operation>,
    warnings: &mut Vec<String>,
    args: &str,
    cd_dir: &Path,
) {
    let args = args.trim();
    if args.is_empty() {
        warnings.push("apply_edit: requires at least <id>".into());
        return;
    }

    let (id, rest) = split_first_token(args);
    let rest = rest.trim();

    // Form 1: just the ID
    if rest.is_empty() {
        ops.push(Operation::ApplyEdit {
            edit_id: id.to_string(),
            path: None,
            symbol_name: None,
            edit_types: None,
        });
        return;
    }

    // Check if rest starts with `[` — Form 2: edit types override
    if rest.starts_with('[') {
        // Parse [type1 type2 ...] — find the closing bracket
        let bracket_content = if let Some(end) = rest.find(']') {
            &rest[1..end]
        } else {
            // No closing bracket — treat everything after `[` as types
            &rest[1..]
        };

        let types: Vec<String> = bracket_content
            .split_whitespace()
            .filter_map(|t| {
                // Also accept comma-separated within brackets
                let t = t.trim_matches(',');
                EditType::from_str(t).map(|et| et.label().to_string())
            })
            .collect();

        if types.is_empty() {
            warnings.push("apply_edit: no valid edit types in brackets".into());
            return;
        }

        ops.push(Operation::ApplyEdit {
            edit_id: id.to_string(),
            path: None,
            symbol_name: None,
            edit_types: Some(types),
        });
        return;
    }

    // Form 3: <path> <symbol>
    let (path_str, symbol) = split_first_token(rest);
    let path = join_path(cd_dir, path_str);
    let symbol = symbol.trim();

    if symbol.is_empty() {
        warnings.push("apply_edit: when providing a path, also provide a symbol name".into());
        return;
    }

    ops.push(Operation::ApplyEdit {
        edit_id: id.to_string(),
        path: Some(path),
        symbol_name: Some(symbol.to_string()),
        edit_types: None,
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

const CTX_OPEN: &str = "<<<";
const CTX_CLOSE: &str = ">>>";

/// Parse `edit_range <path> <symbol> [<<<before>>>] new_content [<<<after>>>]`.
///
/// Before/after context anchors are optional:
/// - Omit `<<<before>>>` → edit starts from the beginning of the body
/// - Omit `<<<after>>>` → edit extends to the end of the body
pub fn handle_edit_range(
    ops: &mut Vec<Operation>,
    warnings: &mut Vec<String>,
    args: &str,
    cd_dir: &Path,
) {
    let args = args.trim();
    if args.is_empty() {
        warnings.push(
            "edit_range: requires <path> <symbol> [<<<before>>>] content [<<<after>>>]".into(),
        );
        return;
    }

    // Extract path
    let (path_str, rest) = split_first_token(args);
    let path = join_path(cd_dir, path_str);
    let rest = rest.trim();

    if rest.is_empty() {
        warnings.push("edit_range: requires <symbol> and content".into());
        return;
    }

    // Extract symbol name
    let (symbol, rest) = split_first_token(rest);
    let rest = rest.trim();

    if rest.is_empty() {
        warnings.push("edit_range: requires content after symbol".into());
        return;
    }

    // Parse the rest: [<<<before>>>] content [<<<after>>>]
    let (before_ctx, new_content, after_ctx) = parse_range_content(rest);

    if new_content.is_empty() {
        warnings.push("edit_range: content cannot be empty".into());
        return;
    }

    ops.push(Operation::EditRange {
        path,
        symbol_name: symbol.to_string(),
        before_ctx,
        after_ctx,
        new_content: unescape_content(&new_content),
        search_dir: dir_string(cd_dir),
    });
}

/// Parse `[<<<before>>>] content [<<<after>>>]` from the remaining args.
///
/// Returns (before_ctx, content, after_ctx).
fn parse_range_content(s: &str) -> (Option<String>, String, Option<String>) {
    let mut before_ctx = None;
    let mut after_ctx = None;
    let mut content_part = s;

    // Check if string starts with <<<before>>>
    if let Some(rest) = content_part.strip_prefix(CTX_OPEN) {
        if let Some(end) = rest.find(CTX_CLOSE) {
            before_ctx = Some(unescape_content(rest[..end].trim()));
            content_part = rest[end + CTX_CLOSE.len()..].trim();
        }
    }

    // Check if remaining string ends with <<<after>>>
    // Find the LAST occurrence of <<< ... >>>
    if let Some(last_open) = content_part.rfind(CTX_OPEN) {
        let after_start = last_open + CTX_OPEN.len();
        if let Some(close_offset) = content_part[after_start..].rfind(CTX_CLOSE) {
            let after_text = &content_part[after_start..after_start + close_offset];
            after_ctx = Some(unescape_content(after_text.trim()));
            content_part = content_part[..last_open].trim();
        }
    }

    (before_ctx, content_part.to_string(), after_ctx)
}
