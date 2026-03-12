/// Extraction utilities for edits: signature/docs ranges, diffing, length checks, IDs.
use std::fmt::Write;

use lsp_types::{Position, Range};

use crate::config::LengthLimits;
use crate::tools::language_specific;

/// Generate a memorable 3-word ID like `blue_fox_river`.
pub(crate) fn word_id() -> String {
    use std::time::SystemTime;
    let t = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos() as u64;
    let mut x = t ^ 0x5DEECE66D;
    x ^= x << 13;
    x ^= x >> 7;
    x ^= x << 17;

    const ADJ: &[&str] = &[
        "red", "blue", "green", "gold", "dark", "bold", "calm", "cold", "warm", "fast", "soft",
        "wild", "tall", "deep", "pale", "keen", "slim", "flat", "raw", "dry",
    ];
    const NOUN: &[&str] = &[
        "fox", "owl", "elm", "oak", "bay", "sky", "sea", "sun", "ash", "ice", "gem", "arc", "bee",
        "fin", "dew", "ore", "rye", "yew", "ink", "fir",
    ];
    const VERB: &[&str] = &[
        "runs", "hops", "dips", "zips", "taps", "nods", "hums", "arcs", "digs", "jets", "maps",
        "sets", "cuts", "fits", "pegs", "tags", "pins", "bows", "rows", "ties",
    ];

    let a = ADJ[(x % ADJ.len() as u64) as usize];
    let b = NOUN[((x >> 8) % NOUN.len() as u64) as usize];
    let c = VERB[((x >> 16) % VERB.len() as u64) as usize];
    format!("{a}_{b}_{c}")
}

/// Extract the signature portion: from first code line to the first `{`.
/// Skips doc comments and attributes that may be included in the symbol range.
pub(crate) fn extract_signature(
    file_lines: &[&str],
    symbol_range: &Range,
    lang: Option<&str>,
) -> (Range, String) {
    let range_start = symbol_range.start.line as usize;
    let end = (symbol_range.end.line as usize).min(file_lines.len().saturating_sub(1));

    // Skip past docs/attrs to find the actual signature start
    let mut start = range_start;
    while start <= end {
        if start < file_lines.len()
            && (language_specific::is_doc_or_attr(lang, file_lines[start])
                || file_lines[start].trim().is_empty())
        {
            start += 1;
        } else {
            break;
        }
    }
    if start > end {
        start = range_start;
    }

    let mut sig_end_line = start;
    let mut sig_end_char = 0u32;

    for line_idx in start..=end {
        if line_idx >= file_lines.len() {
            break;
        }
        let line = file_lines[line_idx];
        if let Some(brace_pos) = line.find('{') {
            sig_end_line = line_idx;
            sig_end_char = brace_pos as u32;
            break;
        }
        if let Some(pos) = line.find(';') {
            sig_end_line = line_idx;
            sig_end_char = pos as u32;
            break;
        }
        sig_end_line = line_idx;
        sig_end_char = line.len() as u32;
    }

    let range = Range {
        start: Position {
            line: start as u32,
            character: 0,
        },
        end: Position {
            line: sig_end_line as u32,
            character: sig_end_char,
        },
    };

    let mut text = String::new();
    for line_idx in start..=sig_end_line {
        if line_idx >= file_lines.len() {
            break;
        }
        let line = file_lines[line_idx];
        if line_idx == sig_end_line {
            text.push_str(&line[..sig_end_char.min(line.len() as u32) as usize]);
        } else {
            text.push_str(line);
            text.push('\n');
        }
    }

    (range, text)
}

/// Extract doc comments for a symbol.
///
/// Handles two cases:
/// 1. LSP range includes docs (rust-analyzer): docs are at the start of the range
/// 2. Docs are above the range: walk backward to find them
pub(crate) fn extract_docs(
    file_lines: &[&str],
    symbol_range: &Range,
    lang: Option<&str>,
) -> (Range, String) {
    let range_start = symbol_range.start.line as usize;
    let range_end = (symbol_range.end.line as usize).min(file_lines.len().saturating_sub(1));

    // Case 1: docs included in the symbol range
    if range_start < file_lines.len()
        && language_specific::is_doc_or_attr(lang, file_lines[range_start])
    {
        let mut doc_end = range_start;
        for i in range_start..=range_end {
            if i >= file_lines.len() {
                break;
            }
            if language_specific::is_doc_or_attr(lang, file_lines[i])
                || file_lines[i].trim().is_empty()
            {
                doc_end = i;
            } else {
                break;
            }
        }
        let mut doc_start = range_start;
        for i in (0..range_start).rev() {
            if language_specific::is_doc_or_attr(lang, file_lines[i]) {
                doc_start = i;
            } else {
                break;
            }
        }
        let range = Range {
            start: Position {
                line: doc_start as u32,
                character: 0,
            },
            end: Position {
                line: doc_end as u32,
                character: file_lines.get(doc_end).map(|l| l.len() as u32).unwrap_or(0),
            },
        };
        let text: String = file_lines[doc_start..=doc_end].join("\n");
        return (range, text);
    }

    // Case 2: Walk backwards from the range start
    let mut doc_start = range_start;
    for i in (0..range_start).rev() {
        if language_specific::is_doc_or_attr(lang, file_lines[i]) {
            doc_start = i;
        } else {
            break;
        }
    }

    if doc_start == range_start {
        let range = Range {
            start: Position {
                line: range_start as u32,
                character: 0,
            },
            end: Position {
                line: range_start as u32,
                character: 0,
            },
        };
        return (range, String::new());
    }

    let doc_end = range_start - 1;
    let range = Range {
        start: Position {
            line: doc_start as u32,
            character: 0,
        },
        end: Position {
            line: doc_end as u32,
            character: file_lines.get(doc_end).map(|l| l.len() as u32).unwrap_or(0),
        },
    };

    let text: String = file_lines[doc_start..=doc_end].join("\n");
    (range, text)
}

/// Count non-empty lines in text.
pub(crate) fn count_non_empty_lines(text: &str) -> usize {
    text.lines().filter(|l| !l.trim().is_empty()).count()
}

/// Check file and function length limits after an edit.
/// Returns warning text (empty if no limits exceeded).
pub(crate) fn check_length_limits(
    file_content: &str,
    symbol_content: Option<&str>,
    symbol_name: &str,
    rel_path: &str,
    limits: &LengthLimits,
) -> String {
    let mut warnings = String::new();

    let file_lines = count_non_empty_lines(file_content);
    if file_lines > limits.file_hard {
        let _ = writeln!(
            warnings,
            "WARNING: {rel_path} has {file_lines} non-empty lines (hard limit: {})",
            limits.file_hard,
        );
    } else if file_lines > limits.file_suggest {
        let _ = writeln!(
            warnings,
            "note: {rel_path} has {file_lines} non-empty lines (suggested limit: {})",
            limits.file_suggest,
        );
    }

    if let Some(sym_content) = symbol_content {
        let fn_lines = count_non_empty_lines(sym_content);
        if fn_lines > limits.fn_hard {
            let _ = writeln!(
                warnings,
                "WARNING: {symbol_name} has {fn_lines} non-empty lines (hard limit: {})",
                limits.fn_hard,
            );
        } else if fn_lines > limits.fn_suggest {
            let _ = writeln!(
                warnings,
                "note: {symbol_name} has {fn_lines} non-empty lines (suggested limit: {})",
                limits.fn_suggest,
            );
        }
    }

    warnings
}

/// Make a path relative to cwd.
pub(crate) fn make_relative(path: &str) -> String {
    let cwd = std::env::current_dir()
        .unwrap_or_default()
        .to_string_lossy()
        .to_string();
    path.strip_prefix(&cwd)
        .unwrap_or(path)
        .trim_start_matches('/')
        .to_string()
}

/// Line-by-line diff using the `similar` crate (patience algorithm).
/// Shows only changed lines with 2 lines of context around each hunk.
pub(crate) fn line_diff(old: &str, new: &str) -> String {
    use similar::TextDiff;

    let diff = TextDiff::from_lines(old, new);
    let mut out = String::new();

    for hunk in diff.unified_diff().context_radius(2).iter_hunks() {
        for change in hunk.iter_changes() {
            let sign = match change.tag() {
                similar::ChangeTag::Delete => "- ",
                similar::ChangeTag::Insert => "+ ",
                similar::ChangeTag::Equal => "  ",
            };
            let value = change.value().trim_end_matches('\n');
            let _ = writeln!(out, "{sign}{value}");
        }
    }

    out
}
