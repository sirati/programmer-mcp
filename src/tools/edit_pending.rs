//! Pending edit management: disambiguation and deferred application.
//!
//! When a symbol edit can't be resolved exactly, candidates are shown and the
//! edit is stored as "pending". The user can then apply it by ID with a
//! corrected path or symbol name.

use std::fmt::Write;
use std::sync::Arc;

use crate::config::LengthLimits;
use crate::lsp::client::{LspClient, LspClientError};
use crate::tools::formatting::uri_to_path;
use crate::tools::symbol_search::{filter_exact_matches, find_symbol_with_fallback};

use super::edit_apply::{apply_file_edit, apply_symbol_edit};
use super::edit_extract::{make_relative, word_id};
use super::edit_types::{EditType, PendingEdit, PendingEdits, UndoStore};

/// Apply an edit that was previously stored as pending (after disambiguation).
///
/// - `path_override` / `symbol_override`: if Some, correct the stored location.
/// - `types_override`: if Some, replace stored edit types.
///
/// If nothing changed from the stored values and resolution still fails,
/// the same ID is re-inserted so the caller can try again.
pub async fn apply_pending_edit(
    client: &Arc<LspClient>,
    edit_id: &str,
    path_override: Option<&str>,
    symbol_override: Option<&str>,
    types_override: Option<&[EditType]>,
    pending: &PendingEdits,
    undo_store: &UndoStore,
    limits: &LengthLimits,
) -> Result<String, LspClientError> {
    let entry = {
        let mut map = pending.lock().await;
        map.remove(edit_id)
    };

    let Some(mut pe) = entry else {
        return Ok(format!("no pending edit with id '{edit_id}'"));
    };

    let path = path_override.unwrap_or(&pe.path);
    let symbol_name = symbol_override.unwrap_or(&pe.symbol_name);

    if let Some(types) = types_override {
        pe.edit_types = types.to_vec();
    }

    // Detect noop: caller provided path/symbol that match what's already stored
    let path_is_noop = path_override
        .map(|p| p == pe.path || pe.path.contains(p) || p.contains(&pe.path))
        .unwrap_or(false);
    let sym_is_noop = symbol_override
        .map(|s| s == pe.symbol_name)
        .unwrap_or(false);

    // For file edits
    if pe.edit_types.len() == 1 && pe.edit_types[0] == EditType::File {
        return apply_file_edit(path, &pe.new_content, undo_store, limits).await;
    }

    // Resolve with effective path/symbol
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
        let mut msg = format!("symbol '{symbol_name}' not found in '{path}'");

        if path_is_noop && sym_is_noop {
            msg.push_str(
                "\nnote: these were already the stored args — \
                 the call was a noop. try correcting the path or symbol.",
            );
        }

        if let Some(p) = path_override {
            pe.path = p.to_string();
        }
        if let Some(s) = symbol_override {
            pe.symbol_name = s.to_string();
        }

        let types_label: Vec<_> = pe.edit_types.iter().map(|t| t.label()).collect();
        let _ = write!(
            msg,
            "\npending edit preserved — use:\n  apply_edit {edit_id} <correct_path> <correct_symbol>\n  \
             (stored types: [{}])",
            types_label.join(" "),
        );

        {
            let mut map = pending.lock().await;
            map.insert(edit_id.to_string(), pe);
        }

        return Ok(msg);
    }

    apply_symbol_edit(
        client,
        &filtered[0],
        &pe.edit_types,
        &pe.new_content,
        pending,
        undo_store,
        true,
        limits,
    )
    .await
}

/// Generate disambiguation response with candidates.
pub async fn disambiguate(
    client: &Arc<LspClient>,
    edit_types: &[EditType],
    path: &str,
    symbol_name: &str,
    new_content: &str,
    search_dir: Option<&str>,
    pending: &PendingEdits,
) -> Result<String, LspClientError> {
    let candidates = find_symbol_with_fallback(client, symbol_name, search_dir).await?;

    if candidates.is_empty() {
        return Ok(format!("edit failed: symbol '{symbol_name}' not found"));
    }

    let id = word_id();
    let mut candidate_list: Vec<(String, String)> = Vec::new();
    let mut output =
        format!("edit of '{symbol_name}' failed — exact match not found\ndid you mean:\n");

    for (i, sym) in candidates.iter().take(10).enumerate() {
        let sym_path = uri_to_path(&sym.location.uri).unwrap_or_default();
        let rel = make_relative(&sym_path);
        let _ = writeln!(output, "  {}. {} {}", i + 1, rel, sym.name);
        candidate_list.push((rel, sym.name.clone()));
    }

    let _ = writeln!(
        output,
        "\nto apply, use:\n  apply_edit {id} <correct_path> <correct_symbol>"
    );

    {
        let mut map = pending.lock().await;
        map.insert(
            id,
            PendingEdit {
                edit_types: edit_types.to_vec(),
                new_content: new_content.to_string(),
                path: path.to_string(),
                symbol_name: symbol_name.to_string(),
                _search_dir: search_dir.map(|s| s.to_string()),
                _candidates: candidate_list,
            },
        );
    }

    Ok(output)
}

/// Find symbols with exact-only matching (no fuzzy fallback).
pub async fn find_symbol_exact(
    client: &Arc<LspClient>,
    name: &str,
) -> Result<Vec<lsp_types::SymbolInformation>, LspClientError> {
    let results = client.symbol_cache().workspace_symbol(client, name).await?;
    Ok(filter_exact_matches(&results, name))
}
