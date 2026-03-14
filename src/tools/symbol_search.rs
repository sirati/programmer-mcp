//! Symbol search with fallback strategies.
//!
//! The primary entry point is [`find_symbol_with_fallback`], which tries a
//! series of progressively fuzzier strategies to locate a symbol by name.

use std::sync::Arc;

use heck::{ToLowerCamelCase, ToPascalCase, ToShoutySnakeCase, ToSnakeCase};
use lsp_types::SymbolInformation;
use tracing::debug;

use super::formatting::is_external_path;
use super::symbol_match::container_matches;
use super::symbol_parent_child::{
    best_fuzzy_matches, name_has_receiver, try_index_parent_doc_child,
    try_parent_child_via_document, try_parent_child_workspace,
};
use super::symbol_walk::try_directory_walk;
use crate::lsp::client::LspClient;

/// Deduplicate symbols that refer to the same definition.
///
/// workspace/symbol and documentSymbol may return the same symbol with
/// slightly different start positions (e.g., one includes the doc comment,
/// the other points at the name). We dedup by checking if two symbols with
/// the same name in the same file have start lines within 5 of each other.
fn dedup_symbols(symbols: Vec<SymbolInformation>) -> Vec<SymbolInformation> {
    let mut result: Vec<SymbolInformation> = Vec::new();
    for sym in symbols {
        let dominated = result.iter().any(|existing| {
            existing.location.uri == sym.location.uri
                && existing.name == sym.name
                && existing
                    .location
                    .range
                    .start
                    .line
                    .abs_diff(sym.location.range.start.line)
                    <= 5
        });
        if !dominated {
            result.push(sym);
        }
    }
    result
}

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
/// Results are deduplicated by location. When both workspace and external results
/// exist for the same symbol name, external results are filtered out.
pub async fn find_symbol_with_fallback(
    client: &Arc<LspClient>,
    name: &str,
    search_dir: Option<&str>,
) -> Result<Vec<SymbolInformation>, crate::lsp::client::LspClientError> {
    let results = find_symbol_inner(client, name, search_dir).await?;
    let deduped = dedup_symbols(results);

    // Prefer workspace results over external (stdlib, registry, nix store)
    let has_workspace = deduped
        .iter()
        .any(|s| !is_external_path(s.location.uri.as_str()));
    if has_workspace {
        Ok(deduped
            .into_iter()
            .filter(|s| !is_external_path(s.location.uri.as_str()))
            .collect())
    } else {
        Ok(deduped)
    }
}

/// Internal symbol search with fallback strategies.
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
async fn find_symbol_inner(
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
                    // Check container_name (set by nested documentSymbol responses)
                    if let Some(c) = s.container_name.as_deref() {
                        return container_matches(c, parent, false);
                    }
                    // For Go-style (*Type).Method names where container_name is None,
                    // check if the full name embeds the parent as a receiver.
                    name_has_receiver(&s.name, parent)
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
