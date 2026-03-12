/// Smart symbol-aware edit command.
///
/// Resolves symbols via LSP, extracts the relevant range (body/signature/docs),
/// applies replacement with indentation normalization, and returns a diff.
/// When exact resolution fails, suggests candidates for disambiguation.
use std::collections::HashMap;
use std::fmt::Write;
use std::sync::Arc;

use lsp_types::{Position, Range, SymbolInformation, Uri};
use tokio::sync::Mutex;
use tracing::debug;

use crate::lsp::client::{LspClient, LspClientError};
use crate::tools::formatting::{
    find_containing_symbol_range, path_to_uri, read_range_from_file, uri_to_path,
};
use crate::tools::indent::{
    base_indent_chars, detect_line_ending, leading_whitespace, normalize_indent,
};
use crate::tools::symbol_search::{filter_exact_matches, find_symbol_with_fallback};

/// Which part of a symbol to edit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EditType {
    Body,
    Signature,
    Docs,
    File,
}

impl EditType {
    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "body" => Some(Self::Body),
            "signature" | "sig" => Some(Self::Signature),
            "docs" | "doc" | "docstring" => Some(Self::Docs),
            "file" => Some(Self::File),
            _ => None,
        }
    }

    pub fn label(&self) -> &'static str {
        match self {
            Self::Body => "body",
            Self::Signature => "signature",
            Self::Docs => "docs",
            Self::File => "file",
        }
    }
}

/// A pending edit waiting for disambiguation.
#[derive(Debug, Clone)]
pub struct PendingEdit {
    pub edit_types: Vec<EditType>,
    pub new_content: String,
    pub search_dir: Option<String>,
    pub candidates: Vec<(String, String)>, // (path, symbol_name)
}

/// Storage for pending edits keyed by short hex ID.
pub type PendingEdits = Arc<Mutex<HashMap<String, PendingEdit>>>;

pub fn new_pending_edits() -> PendingEdits {
    Arc::new(Mutex::new(HashMap::new()))
}

/// Generate a short 4-char hex ID.
fn short_id() -> String {
    use std::time::SystemTime;
    let t = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    format!("{:04x}", (t & 0xFFFF) as u16)
}

/// Find symbols with exact-only matching (no fuzzy fallback).
async fn find_symbol_exact(
    client: &Arc<LspClient>,
    name: &str,
) -> Result<Vec<SymbolInformation>, LspClientError> {
    let results = client.symbol_cache().workspace_symbol(client, name).await?;
    Ok(filter_exact_matches(&results, name))
}

/// Execute a file-only edit (no LSP needed).
pub async fn execute_edit_no_lsp(
    _edit_types: &[EditType],
    path: &str,
    _symbol_name: &str,
    new_content: &str,
    _pending: &PendingEdits,
) -> Result<String, LspClientError> {
    apply_file_edit(path, new_content)
}

/// Execute an edit operation.
///
/// Returns formatted output: either a diff on success, or a disambiguation
/// prompt with candidate suggestions.
pub async fn execute_edit(
    client: &Arc<LspClient>,
    edit_types: &[EditType],
    path: &str,
    symbol_name: &str,
    new_content: &str,
    search_dir: Option<&str>,
    pending: &PendingEdits,
) -> Result<String, LspClientError> {
    // For file-type edits, no symbol resolution needed
    if edit_types.len() == 1 && edit_types[0] == EditType::File {
        return apply_file_edit(path, new_content);
    }

    // Try exact resolution first
    let exact = find_symbol_exact(client, symbol_name).await?;

    // Filter by path if provided
    let filtered: Vec<_> = if path.is_empty() || path == "." {
        exact
    } else {
        exact
            .into_iter()
            .filter(|s| {
                uri_to_path(&s.location.uri)
                    .map(|p| p.contains(path))
                    .unwrap_or(false)
            })
            .collect()
    };

    if filtered.is_empty() {
        // No exact match — find candidates for disambiguation
        return disambiguate(
            client,
            edit_types,
            symbol_name,
            new_content,
            search_dir,
            pending,
        )
        .await;
    }

    // Use the first exact match
    let symbol = &filtered[0];
    apply_symbol_edit(client, symbol, edit_types, new_content).await
}

/// Apply an edit that was previously stored as pending (after disambiguation).
pub async fn apply_pending_edit(
    client: &Arc<LspClient>,
    edit_id: &str,
    path: &str,
    symbol_name: &str,
    pending: &PendingEdits,
) -> Result<String, LspClientError> {
    let entry = {
        let mut map = pending.lock().await;
        map.remove(edit_id)
    };

    let Some(pe) = entry else {
        return Ok(format!("no pending edit with id '{edit_id}'"));
    };

    // For file edits
    if pe.edit_types.len() == 1 && pe.edit_types[0] == EditType::File {
        return apply_file_edit(path, &pe.new_content);
    }

    // Resolve with the corrected path/symbol
    let exact = find_symbol_exact(client, symbol_name).await?;
    let filtered: Vec<_> = if path.is_empty() || path == "." {
        exact
    } else {
        exact
            .into_iter()
            .filter(|s| {
                uri_to_path(&s.location.uri)
                    .map(|p| p.contains(path))
                    .unwrap_or(false)
            })
            .collect()
    };

    if filtered.is_empty() {
        return Ok(format!(
            "symbol '{symbol_name}' not found in '{path}' — edit cancelled"
        ));
    }

    apply_symbol_edit(client, &filtered[0], &pe.edit_types, &pe.new_content).await
}

/// Apply edit types to a resolved symbol.
async fn apply_symbol_edit(
    client: &Arc<LspClient>,
    symbol: &SymbolInformation,
    edit_types: &[EditType],
    new_content: &str,
) -> Result<String, LspClientError> {
    let uri = &symbol.location.uri;
    let path = uri_to_path(uri).unwrap_or_else(|| uri.as_str().to_string());

    // Open file for LSP tracking
    client.open_file(&path).await.ok();

    let full_range = find_containing_symbol_range(client, uri, symbol.location.range.start).await;
    let symbol_range = full_range.unwrap_or(symbol.location.range);

    let file_content = std::fs::read_to_string(&path)
        .map_err(|e| LspClientError::Other(format!("read error: {e}")))?;
    let file_lines: Vec<&str> = file_content.lines().collect();
    let file_line_ending = detect_line_ending(&file_content);

    let mut output = String::new();

    for et in edit_types {
        let (range, old_text) = match et {
            EditType::Body => {
                let text = read_range_from_file(uri, &symbol_range)
                    .map_err(|e| LspClientError::Other(e.to_string()))?;
                (symbol_range, text)
            }
            EditType::Signature => {
                let (r, t) = extract_signature(&file_lines, &symbol_range);
                (r, t)
            }
            EditType::Docs => {
                let (r, t) = extract_docs(&file_lines, &symbol_range);
                (r, t)
            }
            EditType::File => {
                // Shouldn't reach here, handled above
                continue;
            }
        };

        // Determine target indentation from the first line of the old range
        let target_indent = if range.start.line < file_lines.len() as u32 {
            leading_whitespace(file_lines[range.start.line as usize]).to_string()
        } else {
            String::new()
        };

        // Normalize the new content's indentation
        let normalized = normalize_indent(new_content, &target_indent, file_line_ending);

        // Apply the edit, preserving prefix/suffix of partial-line ranges
        let start_line = range.start.line as usize;
        let end_line = (range.end.line as usize).min(file_lines.len().saturating_sub(1));
        let start_char = range.start.character as usize;
        let end_char = range.end.character as usize;

        let prefix = if start_line < file_lines.len() {
            &file_lines[start_line][..start_char.min(file_lines[start_line].len())]
        } else {
            ""
        };
        let suffix = if end_line < file_lines.len() {
            &file_lines[end_line][end_char.min(file_lines[end_line].len())..]
        } else {
            ""
        };

        let mut new_lines: Vec<String> = file_lines.iter().map(|l| l.to_string()).collect();
        let mut replacement_lines: Vec<String> =
            normalized.lines().map(|l| l.to_string()).collect();

        // Attach prefix to first replacement line and suffix to last
        if !prefix.is_empty() {
            if let Some(first) = replacement_lines.first_mut() {
                *first = format!("{prefix}{first}");
            }
        }
        if !suffix.is_empty() {
            if let Some(last) = replacement_lines.last_mut() {
                // Auto-add space before `{` if replacement doesn't end with whitespace
                if suffix.starts_with('{') && !last.ends_with(char::is_whitespace) {
                    last.push(' ');
                }
                last.push_str(suffix);
            } else if !prefix.is_empty() {
                // No replacement lines — create one with prefix+suffix
                replacement_lines.push(format!("{prefix}{suffix}"));
            } else {
                replacement_lines.push(suffix.to_string());
            }
        }

        new_lines.splice(start_line..=end_line, replacement_lines);

        let new_file = new_lines.join(file_line_ending);
        std::fs::write(&path, &new_file)
            .map_err(|e| LspClientError::Other(format!("write error: {e}")))?;

        // Generate diff
        let diff = simple_diff(&old_text, &normalized);
        let rel_path = path
            .strip_prefix(
                &std::env::current_dir()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .to_string(),
            )
            .unwrap_or(&path)
            .trim_start_matches('/');

        let _ = writeln!(
            output,
            "applied edit [{label}] {rel_path} {name} -> diff:\n```diff\n{diff}\n```",
            label = et.label(),
            name = symbol.name,
        );
    }

    Ok(output)
}

/// Apply a whole-file edit (raw text replacement).
fn apply_file_edit(path: &str, new_content: &str) -> Result<String, LspClientError> {
    let abs_path = if std::path::Path::new(path).is_absolute() {
        path.to_string()
    } else {
        std::env::current_dir()
            .unwrap_or_default()
            .join(path)
            .to_string_lossy()
            .to_string()
    };

    let old_content = std::fs::read_to_string(&abs_path)
        .map_err(|e| LspClientError::Other(format!("read error: {e}")))?;

    let line_ending = detect_line_ending(&old_content);
    let normalized = new_content.replace("\r\n", "\n").replace('\n', line_ending);

    std::fs::write(&abs_path, &normalized)
        .map_err(|e| LspClientError::Other(format!("write error: {e}")))?;

    let diff = simple_diff(&old_content, &normalized);
    let rel_path = abs_path
        .strip_prefix(
            &std::env::current_dir()
                .unwrap_or_default()
                .to_string_lossy()
                .to_string(),
        )
        .unwrap_or(&abs_path)
        .trim_start_matches('/');

    Ok(format!(
        "applied edit [file] {rel_path} -> diff:\n```diff\n{diff}\n```"
    ))
}

/// Generate disambiguation response with candidates.
async fn disambiguate(
    client: &Arc<LspClient>,
    edit_types: &[EditType],
    symbol_name: &str,
    new_content: &str,
    search_dir: Option<&str>,
    pending: &PendingEdits,
) -> Result<String, LspClientError> {
    // Use fuzzy search to find candidates
    let candidates = find_symbol_with_fallback(client, symbol_name, search_dir).await?;

    if candidates.is_empty() {
        return Ok(format!("edit failed: symbol '{symbol_name}' not found"));
    }

    let id = short_id();
    let mut candidate_list: Vec<(String, String)> = Vec::new();
    let mut output =
        format!("edit of '{symbol_name}' failed — exact match not found\ndid you mean:\n");

    for (i, sym) in candidates.iter().take(10).enumerate() {
        let path = uri_to_path(&sym.location.uri).unwrap_or_default();
        let rel = path
            .strip_prefix(
                &std::env::current_dir()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .to_string(),
            )
            .unwrap_or(&path)
            .trim_start_matches('/')
            .to_string();
        let _ = writeln!(output, "  {}. {} {}", i + 1, rel, sym.name);
        candidate_list.push((rel, sym.name.clone()));
    }

    let _ = writeln!(
        output,
        "\nto apply, use:\n  apply_edit {id} <correct_path> <correct_symbol>"
    );

    // Store pending edit
    {
        let mut map = pending.lock().await;
        map.insert(
            id,
            PendingEdit {
                edit_types: edit_types.to_vec(),
                new_content: new_content.to_string(),
                search_dir: search_dir.map(|s| s.to_string()),
                candidates: candidate_list,
            },
        );
    }

    Ok(output)
}

/// Extract the signature portion: from first code line to the first `{`.
/// Skips doc comments and attributes that may be included in the symbol range.
fn extract_signature(file_lines: &[&str], symbol_range: &Range) -> (Range, String) {
    let range_start = symbol_range.start.line as usize;
    let end = (symbol_range.end.line as usize).min(file_lines.len().saturating_sub(1));

    // Skip past docs/attrs to find the actual signature start
    let mut start = range_start;
    while start <= end {
        if start < file_lines.len()
            && (is_doc_or_attr(file_lines[start]) || file_lines[start].trim().is_empty())
        {
            start += 1;
        } else {
            break;
        }
    }
    if start > end {
        start = range_start; // fallback
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
        // Also check for `:` (Python, etc.) or `;` (declarations without body)
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

/// Check if a line looks like a doc comment or attribute (not code).
fn is_doc_or_attr(line: &str) -> bool {
    let trimmed = line.trim();
    trimmed.starts_with("///")
        || trimmed.starts_with("//!")
        || trimmed.starts_with("/**")
        || trimmed.starts_with("* ")
        || trimmed.starts_with("*/")
        || trimmed == "*"
        || trimmed.starts_with("\"\"\"")
        || trimmed.starts_with("'''")
        || (trimmed.starts_with('#') && trimmed.contains('['))
}

/// Extract doc comments for a symbol.
///
/// Handles two cases:
/// 1. LSP range includes docs (rust-analyzer): docs are at the start of the range
/// 2. Docs are above the range: walk backward to find them
fn extract_docs(file_lines: &[&str], symbol_range: &Range) -> (Range, String) {
    let range_start = symbol_range.start.line as usize;
    let range_end = (symbol_range.end.line as usize).min(file_lines.len().saturating_sub(1));

    // Case 1: Check if docs are included in the symbol range
    // (rust-analyzer includes /// comments in the symbol range)
    if range_start < file_lines.len() && is_doc_or_attr(file_lines[range_start]) {
        let mut doc_end = range_start;
        for i in range_start..=range_end {
            if i >= file_lines.len() {
                break;
            }
            if is_doc_or_attr(file_lines[i]) || file_lines[i].trim().is_empty() {
                doc_end = i;
            } else {
                break;
            }
        }
        // Also walk backward in case there are more docs/attrs above range
        let mut doc_start = range_start;
        for i in (0..range_start).rev() {
            if is_doc_or_attr(file_lines[i]) {
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
        if is_doc_or_attr(file_lines[i]) {
            doc_start = i;
        } else {
            break;
        }
    }

    if doc_start == range_start {
        // No docs found — return empty range at insertion point
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

    let doc_end = range_start - 1; // inclusive
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

/// Simple line-by-line diff (removed/added lines).
fn simple_diff(old: &str, new: &str) -> String {
    let old_lines: Vec<&str> = old.lines().collect();
    let new_lines: Vec<&str> = new.lines().collect();

    // If both are short, just show - and +
    if old_lines.len() + new_lines.len() <= 40 {
        let mut diff = String::new();
        for line in &old_lines {
            let _ = writeln!(diff, "- {line}");
        }
        for line in &new_lines {
            let _ = writeln!(diff, "+ {line}");
        }
        return diff;
    }

    // For longer diffs, use a simple LCS-based approach
    let mut diff = String::new();
    let mut i = 0;
    let mut j = 0;

    while i < old_lines.len() && j < new_lines.len() {
        if old_lines[i] == new_lines[j] {
            let _ = writeln!(diff, "  {}", old_lines[i]);
            i += 1;
            j += 1;
        } else {
            // Find how far the difference extends
            let _ = writeln!(diff, "- {}", old_lines[i]);
            i += 1;
        }
    }

    while i < old_lines.len() {
        let _ = writeln!(diff, "- {}", old_lines[i]);
        i += 1;
    }
    while j < new_lines.len() {
        let _ = writeln!(diff, "+ {}", new_lines[j]);
        j += 1;
    }

    diff
}
