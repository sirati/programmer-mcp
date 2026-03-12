/// Core edit application logic: applying symbol and file edits.
use std::fmt::Write;
use std::sync::Arc;

use lsp_types::SymbolInformation;

use crate::config::LengthLimits;
use crate::lsp::client::{LspClient, LspClientError};
use crate::tools::formatting::{find_containing_symbol_range, read_range_from_file, uri_to_path};
use crate::tools::indent::{detect_line_ending, leading_whitespace, normalize_indent};
use crate::tools::language_specific;

use super::edit::{EditType, PendingEdit, PendingEdits, UndoEntry, UndoStore};
use super::edit_extract::{
    check_length_limits, extract_docs, extract_signature, line_diff, make_relative, word_id,
};

/// Apply edit types to a resolved symbol.
/// `force` skips the signature-in-body detection (used when applying pending edits).
pub(crate) async fn apply_symbol_edit(
    client: &Arc<LspClient>,
    symbol: &SymbolInformation,
    edit_types: &[EditType],
    new_content: &str,
    pending: &PendingEdits,
    undo_store: &UndoStore,
    force: bool,
    limits: &LengthLimits,
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
    let lang = language_specific::lang_from_path(&path);

    // ── Signature-in-body detection ─────────────────────────────────────────
    if !force && edit_types == [EditType::Body] {
        let (_sig_range, old_sig) = extract_signature(&file_lines, &symbol_range, lang);
        let new_first_line = new_content.lines().next().unwrap_or("").trim();
        let old_sig_first = old_sig.lines().next().unwrap_or("").trim();

        if !new_first_line.is_empty()
            && !old_sig_first.is_empty()
            && language_specific::looks_like_signature(lang, new_first_line)
            && new_first_line != old_sig_first
        {
            let id = word_id();
            let rel_path = make_relative(&path);
            let msg = format!(
                "detected a signature change at the start of a body-only edit!\n\
                 please use `edit body,signature` in the future.\n\
                 to apply this edit anyway:\n  \
                 apply_edit {id} [signature body]\n",
            );

            {
                let mut map = pending.lock().await;
                map.insert(
                    id,
                    PendingEdit {
                        edit_types: vec![EditType::Body],
                        new_content: new_content.to_string(),
                        path: rel_path,
                        symbol_name: symbol.name.clone(),
                        search_dir: None,
                        candidates: vec![],
                    },
                );
            }

            return Ok(msg);
        }
    }

    let mut output = String::new();

    for et in edit_types {
        let (range, old_text) = match et {
            EditType::Body => {
                let text = read_range_from_file(uri, &symbol_range)
                    .map_err(|e| LspClientError::Other(e.to_string()))?;
                (symbol_range, text)
            }
            EditType::Signature => {
                let (r, t) = extract_signature(&file_lines, &symbol_range, lang);
                (r, t)
            }
            EditType::Docs => {
                let (r, t) = extract_docs(&file_lines, &symbol_range, lang);
                (r, t)
            }
            EditType::File => {
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

        if !prefix.is_empty() {
            if let Some(first) = replacement_lines.first_mut() {
                *first = format!("{prefix}{first}");
            }
        }
        if !suffix.is_empty() {
            if let Some(last) = replacement_lines.last_mut() {
                if suffix.starts_with('{') && !last.ends_with(char::is_whitespace) {
                    last.push(' ');
                }
                last.push_str(suffix);
            } else if !prefix.is_empty() {
                replacement_lines.push(format!("{prefix}{suffix}"));
            } else {
                replacement_lines.push(suffix.to_string());
            }
        }

        new_lines.splice(start_line..=end_line, replacement_lines);

        let new_file = new_lines.join(file_line_ending);
        std::fs::write(&path, &new_file)
            .map_err(|e| LspClientError::Other(format!("write error: {e}")))?;

        let diff = line_diff(&old_text, &normalized);
        let rel_path = make_relative(&path);

        let _ = writeln!(
            output,
            "applied edit [{label}] {rel_path} {name} -> diff:\n```diff\n{diff}\n```",
            label = et.label(),
            name = symbol.name,
        );
    }

    // Check length limits on the resulting file
    let final_content = std::fs::read_to_string(&path)
        .map_err(|e| LspClientError::Other(format!("read error: {e}")))?;

    let sym_content = read_range_from_file(uri, &symbol_range).ok();
    let rel_path = make_relative(&path);
    let warnings = check_length_limits(
        &final_content,
        sym_content.as_deref(),
        &symbol.name,
        &rel_path,
        limits,
    );
    if !warnings.is_empty() {
        output.push_str(&warnings);
    }

    // Store undo entry: old file content -> current file content
    let undo_id = word_id();
    {
        let mut map = undo_store.lock().await;
        map.insert(
            undo_id.clone(),
            UndoEntry {
                file_path: path.clone(),
                old_content: file_content.clone(),
                new_content: final_content,
            },
        );
    }
    let _ = writeln!(output, "undo with: undo {undo_id}");

    Ok(output)
}

/// Find context anchor in lines. Returns the line index where the context ends (exclusive).
/// Matching is whitespace-insensitive (trimmed comparison).
fn find_context_anchor(lines: &[&str], ctx_lines: &[&str], search_from: usize) -> Vec<usize> {
    if ctx_lines.is_empty() {
        return vec![search_from];
    }
    let first_ctx = ctx_lines[0].trim();
    let mut matches = Vec::new();

    for i in search_from..lines.len() {
        if lines[i].trim() == first_ctx {
            // Check if remaining context lines match
            let mut all_match = true;
            for (j, ctx) in ctx_lines.iter().enumerate().skip(1) {
                let li = i + j;
                if li >= lines.len() || lines[li].trim() != ctx.trim() {
                    all_match = false;
                    break;
                }
            }
            if all_match {
                matches.push(i + ctx_lines.len()); // line after the last context line
            }
        }
    }
    matches
}

/// Find context anchor searching backwards. Returns the line index where the context starts.
fn find_context_anchor_end(lines: &[&str], ctx_lines: &[&str], search_before: usize) -> Vec<usize> {
    if ctx_lines.is_empty() {
        return vec![search_before];
    }
    let first_ctx = ctx_lines[0].trim();
    let mut matches = Vec::new();

    for i in 0..search_before {
        if lines[i].trim() == first_ctx {
            let mut all_match = true;
            for (j, ctx) in ctx_lines.iter().enumerate().skip(1) {
                let li = i + j;
                if li >= lines.len() || lines[li].trim() != ctx.trim() {
                    all_match = false;
                    break;
                }
            }
            if all_match {
                matches.push(i); // first line of the after-context
            }
        }
    }
    matches
}

/// Apply a targeted range edit within a symbol's body using before/after context anchors.
pub(crate) async fn apply_range_edit(
    client: &Arc<LspClient>,
    symbol: &SymbolInformation,
    before_ctx: Option<&str>,
    after_ctx: Option<&str>,
    new_content: &str,
    undo_store: &UndoStore,
    limits: &LengthLimits,
) -> Result<String, LspClientError> {
    let uri = &symbol.location.uri;
    let path = uri_to_path(uri).unwrap_or_else(|| uri.as_str().to_string());

    client.open_file(&path).await.ok();

    let full_range = find_containing_symbol_range(client, uri, symbol.location.range.start).await;
    let symbol_range = full_range.unwrap_or(symbol.location.range);

    let file_content = std::fs::read_to_string(&path)
        .map_err(|e| LspClientError::Other(format!("read error: {e}")))?;
    let file_lines: Vec<&str> = file_content.lines().collect();
    let file_line_ending = detect_line_ending(&file_content);

    let body_start = symbol_range.start.line as usize;
    let body_end = (symbol_range.end.line as usize + 1).min(file_lines.len());
    let body_lines = &file_lines[body_start..body_end];

    // Find before anchor (relative to body)
    let before_lines: Vec<&str> = before_ctx.map(|s| s.lines().collect()).unwrap_or_default();
    let before_matches = if before_ctx.is_some() {
        find_context_anchor(body_lines, &before_lines, 0)
            .into_iter()
            .map(|i| i + body_start) // convert to absolute line index
            .collect::<Vec<_>>()
    } else {
        vec![body_start] // from start of body
    };

    if before_matches.is_empty() {
        return Ok(format!(
            "edit_range failed: before context not found in {} {}",
            make_relative(&path),
            symbol.name
        ));
    }

    // For each before match, find after anchor
    let mut ranges = Vec::new();
    for &start in &before_matches {
        let after_lines: Vec<&str> = after_ctx.map(|s| s.lines().collect()).unwrap_or_default();
        let after_matches = if after_ctx.is_some() {
            find_context_anchor_end(body_lines, &after_lines, body_lines.len())
                .into_iter()
                .map(|i| i + body_start)
                .filter(|&end| end > start) // after must be after before
                .collect::<Vec<_>>()
        } else {
            vec![body_end] // to end of body
        };

        for &end in &after_matches {
            ranges.push((start, end));
        }
    }

    if ranges.is_empty() {
        return Ok(format!(
            "edit_range failed: after context not found in {} {}",
            make_relative(&path),
            symbol.name
        ));
    }

    if ranges.len() > 1 {
        // Multiple matches — present disambiguation
        let mut msg = format!(
            "edit_range: {} matches found in {} {}\n",
            ranges.len(),
            make_relative(&path),
            symbol.name
        );
        for (i, (s, e)) in ranges.iter().enumerate() {
            let preview: String = file_lines[*s..*e]
                .iter()
                .take(3)
                .map(|l| l.trim())
                .collect::<Vec<_>>()
                .join(" | ");
            let _ = writeln!(msg, "  {}: L{}-L{} ({preview}...)", i + 1, s + 1, e);
        }
        msg.push_str("please provide more specific context to narrow down the match");
        return Ok(msg);
    }

    // Single match — apply the edit
    let (replace_start, replace_end) = ranges[0];
    let old_text = file_lines[replace_start..replace_end].join("\n");

    // Detect target indent from the first line of the replace range
    let target_indent = if replace_start < file_lines.len() {
        leading_whitespace(file_lines[replace_start]).to_string()
    } else {
        String::new()
    };

    let normalized = normalize_indent(new_content, &target_indent, file_line_ending);

    let mut new_lines: Vec<String> = file_lines.iter().map(|l| l.to_string()).collect();
    let replacement: Vec<String> = normalized.lines().map(|l| l.to_string()).collect();
    new_lines.splice(replace_start..replace_end, replacement);

    let new_file = new_lines.join(file_line_ending);
    std::fs::write(&path, &new_file)
        .map_err(|e| LspClientError::Other(format!("write error: {e}")))?;

    let diff = line_diff(&old_text, &normalized);
    let rel_path = make_relative(&path);

    let mut output = format!(
        "applied edit_range {rel_path} {} L{}-L{} -> diff:\n```diff\n{diff}\n```\n",
        symbol.name,
        replace_start + 1,
        replace_end,
    );

    // Length limits check
    let final_content = std::fs::read_to_string(&path)
        .map_err(|e| LspClientError::Other(format!("read error: {e}")))?;
    let sym_content = read_range_from_file(uri, &symbol_range).ok();
    let warnings = check_length_limits(
        &final_content,
        sym_content.as_deref(),
        &symbol.name,
        &rel_path,
        limits,
    );
    if !warnings.is_empty() {
        output.push_str(&warnings);
    }

    // Store undo entry
    let undo_id = word_id();
    {
        let mut map = undo_store.lock().await;
        map.insert(
            undo_id.clone(),
            UndoEntry {
                file_path: path.clone(),
                old_content: file_content,
                new_content: new_file,
            },
        );
    }
    let _ = writeln!(output, "undo with: undo {undo_id}");

    Ok(output)
}

/// Apply a whole-file edit (raw text replacement).
pub(crate) async fn apply_file_edit(
    path: &str,
    new_content: &str,
    undo_store: &UndoStore,
    limits: &LengthLimits,
) -> Result<String, LspClientError> {
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

    let diff = line_diff(&old_content, &normalized);
    let rel_path = make_relative(&abs_path);

    let mut result = format!("applied edit [file] {rel_path} -> diff:\n```diff\n{diff}\n```\n");

    let warnings = check_length_limits(&normalized, None, "", &rel_path, limits);
    if !warnings.is_empty() {
        result.push_str(&warnings);
    }

    // Store undo entry
    let undo_id = word_id();
    {
        let mut map = undo_store.lock().await;
        map.insert(
            undo_id.clone(),
            UndoEntry {
                file_path: abs_path,
                old_content,
                new_content: normalized,
            },
        );
    }
    let _ = writeln!(result, "undo with: undo {undo_id}");

    Ok(result)
}
