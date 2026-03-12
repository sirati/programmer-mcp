//! Parent.child resolution helpers for dotted symbol names.
//!
//! These functions implement the various strategies for resolving `Parent.child`
//! style names — index-based container filtering, workspace/symbol lookups with
//! container matching, and documentSymbol scanning of parent files.

use std::sync::Arc;

use lsp_types::{SymbolInformation, Uri};
use strsim::jaro_winkler;
use tracing::debug;

use super::formatting::uri_to_path;
use super::symbol_match::{child_name_matches, collect_doc_symbol_matches, container_matches};
use crate::lsp::client::LspClient;

/// Check if a symbol name contains a receiver/qualifier matching the parent.
/// Handles Go-style `(*Client).Call`, `(Client).Call` etc.
pub fn name_has_receiver(name: &str, parent: &str) -> bool {
    name.contains(&format!("({parent}).")) || name.contains(&format!("(*{parent})."))
}

/// Sort by Jaro-Winkler score descending, returning those above 0.8.
pub fn best_fuzzy_matches(symbols: Vec<SymbolInformation>, query: &str) -> Vec<SymbolInformation> {
    let mut scored: Vec<(f64, SymbolInformation)> = symbols
        .into_iter()
        .map(|s| (jaro_winkler(&s.name, query), s))
        .collect();
    scored.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
    scored
        .into_iter()
        .filter(|(score, _)| *score > 0.8)
        .map(|(_, s)| s)
        .collect()
}

/// Find parent in symbol index, then scan its file(s) for child via documentSymbol.
pub async fn try_index_parent_doc_child(
    client: &Arc<LspClient>,
    parent: &str,
    child: &str,
) -> Result<Option<Vec<SymbolInformation>>, crate::lsp::client::LspClientError> {
    let parent_from_index = client.symbol_cache().exact_search(parent).await;
    if parent_from_index.is_empty() {
        return Ok(None);
    }

    let mut uris: Vec<Uri> = Vec::new();
    for sym in &parent_from_index {
        let uri = sym.location.uri.clone();
        if !uris.contains(&uri) {
            uris.push(uri);
        }
    }

    for uri in &uris {
        if let Some(path) = uri_to_path(uri) {
            let _ = client.open_file(&path).await;
        }
        let doc_symbols = match client.document_symbol(uri).await {
            Ok(s) => s,
            Err(_) => continue,
        };
        let exact = collect_doc_symbol_matches(&doc_symbols, uri, child, false);
        if !exact.is_empty() {
            debug!(
                parent,
                child, "parent.child: found via index parent + doc child"
            );
            return Ok(Some(exact));
        }
    }

    Ok(None)
}

/// Attempt to match `child` symbols from workspace/symbol whose `container_name`
/// matches `parent`.
pub async fn try_parent_child_workspace(
    client: &Arc<LspClient>,
    parent: &str,
    child: &str,
    fuzzy_parent: bool,
    fuzzy_child: bool,
) -> Result<Option<Vec<SymbolInformation>>, crate::lsp::client::LspClientError> {
    let results = client
        .symbol_cache()
        .workspace_symbol(client, child)
        .await?;

    let matches: Vec<SymbolInformation> = results
        .into_iter()
        .filter(|s| child_name_matches(&s.name, child, fuzzy_child))
        .filter(|s| {
            s.container_name
                .as_deref()
                .map(|c| container_matches(c, parent, fuzzy_parent))
                .unwrap_or(false)
        })
        .collect();

    if matches.is_empty() {
        return Ok(None);
    }
    debug!(
        parent,
        child,
        fuzzy_parent,
        fuzzy_child,
        count = matches.len(),
        "parent.child match via workspace/symbol"
    );
    Ok(Some(matches))
}

/// Find the parent symbol first (via workspace/symbol), then search for `child`
/// inside the parent's source file using textDocument/documentSymbol.
pub async fn try_parent_child_via_document(
    client: &Arc<LspClient>,
    parent: &str,
    child: &str,
    fuzzy_parent: bool,
    fuzzy_child: bool,
) -> Result<Option<Vec<SymbolInformation>>, crate::lsp::client::LspClientError> {
    let parent_results = client
        .symbol_cache()
        .workspace_symbol(client, parent)
        .await?;
    let parent_symbols: Vec<_> = parent_results
        .iter()
        .filter(|s| child_name_matches(&s.name, parent, fuzzy_parent))
        .collect();

    if parent_symbols.is_empty() {
        return Ok(None);
    }

    // Collect unique URIs from parent symbols
    let mut uris: Vec<Uri> = Vec::new();
    for sym in &parent_symbols {
        let uri = sym.location.uri.clone();
        if !uris.contains(&uri) {
            uris.push(uri);
        }
    }

    let mut all_matches: Vec<SymbolInformation> = Vec::new();
    for uri in &uris {
        if let Some(path) = uri_to_path(uri) {
            let _ = client.open_file(&path).await;
        }
        let doc_symbols = match client.document_symbol(uri).await {
            Ok(s) => s,
            Err(_) => continue,
        };
        all_matches.extend(collect_doc_symbol_matches(
            &doc_symbols,
            uri,
            child,
            fuzzy_child,
        ));
    }

    if all_matches.is_empty() {
        return Ok(None);
    }
    debug!(
        parent,
        child,
        fuzzy_parent,
        fuzzy_child,
        count = all_matches.len(),
        "parent.child match via document/symbol"
    );
    Ok(Some(all_matches))
}
