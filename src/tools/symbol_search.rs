//! Symbol search with fallback strategies.
//!
//! The primary entry point is [`find_symbol_with_fallback`], which tries a
//! series of progressively fuzzier strategies to locate a symbol by name.

use std::sync::Arc;

use heck::{ToLowerCamelCase, ToPascalCase, ToShoutySnakeCase, ToSnakeCase};
use lsp_types::{SymbolInformation, Uri};
use strsim::jaro_winkler;
use tracing::debug;

use super::formatting::uri_to_path;
use super::symbol_match::{child_name_matches, collect_doc_symbol_matches, container_matches};
use super::symbol_walk::try_directory_walk;
use crate::lsp::client::LspClient;

/// Generate case variations of a symbol name for fuzzy matching.
pub fn case_variations(name: &str) -> Vec<String> {
    let mut variants = vec![name.to_string()];

    for v in [
        name.to_snake_case(),
        name.to_lower_camel_case(),
        name.to_pascal_case(),
        name.to_shouty_snake_case(),
    ] {
        if v != name && !variants.contains(&v) {
            variants.push(v);
        }
    }

    variants
}

/// Filter symbol results to exact name matches, handling qualified names.
pub fn filter_exact_matches(symbols: &[SymbolInformation], name: &str) -> Vec<SymbolInformation> {
    symbols
        .iter()
        .filter(|s| {
            if name.contains('.') {
                s.name == name
            } else {
                s.name == name
                    || s.name.ends_with(&format!(".{name}"))
                    || s.name.ends_with(&format!("::{name}"))
            }
        })
        .cloned()
        .collect()
}

/// Search for a symbol, trying index-based lookups first, then LSP round-trips.
///
/// For dotted names (e.g. `Client.send`), resolution order:
///   1. Index: child by name + container filter (O(1), most precise)
///   2. Index: parent lookup + documentSymbol scan for child
///   3-4. workspace/symbol(child) with exact/fuzzy parent container
///   5-8. workspace/symbol(parent) + documentSymbol for child
///   9. child-only exact (workspace/symbol)
///   10. child-only fuzzy (workspace/symbol)
///
/// For plain names, resolution order:
///   1. Symbol index exact + case variations
///   2. workspace/symbol exact + case variations + fuzzy
///   3. Nucleo fuzzy index
///   4. Directory walk fallback
pub async fn find_symbol_with_fallback(
    client: &Arc<LspClient>,
    name: &str,
    search_dir: Option<&str>,
) -> Result<Vec<SymbolInformation>, crate::lsp::client::LspClientError> {
    // ── dotted parent.child resolution ──────────────────────────────────────
    if let Some(dot_pos) = name.rfind('.') {
        let parent = &name[..dot_pos];
        let child = &name[dot_pos + 1..];

        debug!(parent, child, "trying parent.child resolution");

        // ── Index-based lookups (fast, O(1)) ────────────────────────────────

        // 1. Index: child by name, filter by container matching parent
        let child_from_index = client.symbol_cache().exact_search(child).await;
        if !child_from_index.is_empty() {
            let filtered: Vec<_> = child_from_index
                .into_iter()
                .filter(|s| {
                    s.container_name
                        .as_deref()
                        .map(|c| container_matches(c, parent, false))
                        .unwrap_or(false)
                })
                .collect();
            if !filtered.is_empty() {
                debug!(
                    parent,
                    child, "parent.child: found via index child + container filter"
                );
                return Ok(filtered);
            }
        }

        // 2. Index: find parent, scan its file(s) for child via documentSymbol
        if let Some(found) = try_index_parent_doc_child(client, parent, child).await? {
            return Ok(found);
        }

        // ── workspace/symbol-based lookups ──────────────────────────────────

        // 3 & 4: workspace/symbol(child) with exact/fuzzy parent container
        for fuzzy_parent in [false, true] {
            if let Some(found) =
                try_parent_child_workspace(client, parent, child, fuzzy_parent, false).await?
            {
                return Ok(found);
            }
        }

        // 5-8: find parent via workspace/symbol, then document/symbol
        for fuzzy_parent in [false, true] {
            for fuzzy_child in [false, true] {
                if let Some(found) =
                    try_parent_child_via_document(client, parent, child, fuzzy_parent, fuzzy_child)
                        .await?
                {
                    return Ok(found);
                }
            }
        }

        // 9: child-only exact
        let child_results = client
            .symbol_cache()
            .workspace_symbol(client, child)
            .await?;
        let exact = filter_exact_matches(&child_results, child);
        if !exact.is_empty() {
            debug!(
                child,
                "parent.child: found child via exact child-only search"
            );
            return Ok(exact);
        }

        // 10: child-only fuzzy
        if !child_results.is_empty() {
            let good = best_fuzzy_matches(child_results, child);
            if !good.is_empty() {
                debug!(
                    child,
                    "parent.child: found child via fuzzy child-only search"
                );
                return Ok(good);
            }
        }

        // Fall through to plain search on the full name.
    }

    // ── plain (non-dotted) resolution ────────────────────────────────────────

    // ── 1. Symbol index (fast, local) ───────────────────────────────────────
    let exact_from_index = client.symbol_cache().exact_search(name).await;
    if !exact_from_index.is_empty() {
        debug!(
            name,
            count = exact_from_index.len(),
            "found via symbol index (exact)"
        );
        return Ok(exact_from_index);
    }

    // Case variations on the index
    for variant in case_variations(name).into_iter().skip(1) {
        let exact = client.symbol_cache().exact_search(&variant).await;
        if !exact.is_empty() {
            debug!(name, variant = %variant, "found via symbol index (case variation)");
            return Ok(exact);
        }
    }

    // ── 2. workspace/symbol (LSP round-trip) ────────────────────────────────
    let results = client.symbol_cache().workspace_symbol(client, name).await?;
    let exact = filter_exact_matches(&results, name);
    if !exact.is_empty() {
        return Ok(exact);
    }

    // Case variations via workspace/symbol
    for variant in case_variations(name).into_iter().skip(1) {
        debug!(original = name, variant = %variant, "trying case variation");
        let results = client
            .symbol_cache()
            .workspace_symbol(client, &variant)
            .await?;
        let exact = filter_exact_matches(&results, &variant);
        if !exact.is_empty() {
            return Ok(exact);
        }
    }

    // Fuzzy: use original query results and find best matches by similarity
    if !results.is_empty() {
        let good = best_fuzzy_matches(results, name);
        if !good.is_empty() {
            return Ok(good);
        }
    }

    // ── 3. Cached fuzzy index fallback ──────────────────────────────────────
    let fuzzy_results = client.symbol_cache().fuzzy_search(name, 10).await;
    if !fuzzy_results.is_empty() {
        let exact = filter_exact_matches(&fuzzy_results, name);
        if !exact.is_empty() {
            debug!(name, "found via cached fuzzy index (exact)");
            return Ok(exact);
        }
        debug!(
            name,
            count = fuzzy_results.len(),
            "found via cached fuzzy index"
        );
        return Ok(fuzzy_results);
    }

    // ── 4. Directory-walk fallback ──────────────────────────────────────────
    if let Some(dir) = search_dir {
        if let Some(found) = try_directory_walk(client, name, dir).await? {
            return Ok(found);
        }
    }

    Ok(vec![])
}

// ── private helpers ───────────────────────────────────────────────────────────

/// Sort by Jaro-Winkler score descending, returning those above 0.8.
fn best_fuzzy_matches(symbols: Vec<SymbolInformation>, query: &str) -> Vec<SymbolInformation> {
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
async fn try_index_parent_doc_child(
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
async fn try_parent_child_workspace(
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
async fn try_parent_child_via_document(
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
