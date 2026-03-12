/// Range-based edit application using before/after context anchors.
use std::fmt::Write;
use std::sync::Arc;

use lsp_types::SymbolInformation;

use crate::config::LengthLimits;
use crate::lsp::client::{LspClient, LspClientError};
use crate::tools::formatting::{find_containing_symbol_range, read_range_from_file, uri_to_path};
use crate::tools::indent::{detect_line_ending, leading_whitespace, normalize_indent};

use super::edit::{UndoEntry, UndoStore};
use super::edit_extract::{check_length_limits, line_diff, make_relative, word_id};

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
            let mut all_match = true;
            for (j, ctx) in ctx_lines.iter().enumerate().skip(1) {
                let li = i + j;
                if li >= lines.len() || lines[li].trim() != ctx.trim() {
                    all_match = false;
                    break;
                }
            }
            if all_match {
                matches.push(i + ctx_lines.len());
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
                matches.push(i);
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
            .map(|i| i + body_start)
            .collect::<Vec<_>>()
    } else {
        vec![body_start]
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
                .filter(|&end| end > start)
                .collect::<Vec<_>>()
        } else {
            vec![body_end]
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
