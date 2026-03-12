use std::sync::Arc;

use heck::{ToLowerCamelCase, ToPascalCase, ToShoutySnakeCase, ToSnakeCase};
use lsp_types::{DocumentSymbolResponse, Location, SymbolInformation, Uri};
use strsim::jaro_winkler;
use tracing::debug;

use super::formatting::uri_to_path;
use crate::lsp::client::LspClient;

/// Generate case variations of a symbol name for fuzzy matching.
pub fn case_variations(name: &str) -> Vec<String> {
    let mut variants = vec![name.to_string()];

    let snake = name.to_snake_case();
    let camel = name.to_lower_camel_case();
    let pascal = name.to_pascal_case();
    let screaming = name.to_shouty_snake_case();

    for v in [snake, camel, pascal, screaming] {
        if v != name && !variants.contains(&v) {
            variants.push(v);
        }
    }

    variants
}

/// Search for a symbol using workspace/symbol, trying case variations if needed.
///
/// When `name` contains `.` (e.g. `RelayChannel.relay`) a multi-step parent/child
/// fallback is attempted before the plain fuzzy search:
///   1. workspace/symbol(child) filtered by exact parent container name
///   2. workspace/symbol(child) filtered by fuzzy parent container name
///   3. document/symbol on parent's file, exact child in document
///   4. document/symbol on parent's file, fuzzy child in document
///   5. fuzzy parent lookup, document/symbol for exact child
///   6. child-only exact (workspace/symbol)
///   7. child-only fuzzy (workspace/symbol)
pub async fn find_symbol_with_fallback(
    client: &Arc<LspClient>,
    name: &str,
) -> Result<Vec<SymbolInformation>, crate::lsp::client::LspClientError> {
    // ── dotted parent.child resolution ──────────────────────────────────────
    if let Some(dot_pos) = name.rfind('.') {
        let parent = &name[..dot_pos];
        let child = &name[dot_pos + 1..];

        debug!(parent, child, "trying parent.child resolution");

        // Step 1: workspace/symbol(child) + exact parent container
        if let Some(found) = try_parent_child_workspace(client, parent, child, false, false).await?
        {
            return Ok(found);
        }
        // Step 2: workspace/symbol(child) + fuzzy parent container
        if let Some(found) = try_parent_child_workspace(client, parent, child, true, false).await? {
            return Ok(found);
        }
        // Step 3 & 4: find parent via workspace/symbol, then document/symbol on its file
        // (exact parent first, then fuzzy parent)
        for fuzzy_parent in [false, true] {
            if let Some(found) =
                try_parent_child_via_document(client, parent, child, fuzzy_parent, false).await?
            {
                return Ok(found);
            }
            if let Some(found) =
                try_parent_child_via_document(client, parent, child, fuzzy_parent, true).await?
            {
                return Ok(found);
            }
        }

        // Step 6: child-only exact (ignore parent entirely)
        let child_results = client.workspace_symbol(child).await?;
        let exact = filter_exact_matches(&child_results, child);
        if !exact.is_empty() {
            debug!(
                child,
                "parent.child: found child via exact child-only search"
            );
            return Ok(exact);
        }
        // Step 7: child-only fuzzy
        if !child_results.is_empty() {
            let mut scored: Vec<(f64, SymbolInformation)> = child_results
                .into_iter()
                .map(|s| (jaro_winkler(&s.name, child), s))
                .collect();
            scored.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
            let good: Vec<_> = scored
                .into_iter()
                .filter(|(score, _)| *score > 0.8)
                .map(|(_, s)| s)
                .collect();
            if !good.is_empty() {
                debug!(
                    child,
                    "parent.child: found child via fuzzy child-only search"
                );
                return Ok(good);
            }
        }

        // Nothing found via dotted resolution – fall through to plain search on
        // the full name so callers get a consistent "not found" message.
    }

    // ── plain (non-dotted) resolution ────────────────────────────────────────

    // First try exact name
    let results = client.workspace_symbol(name).await?;
    let exact = filter_exact_matches(&results, name);
    if !exact.is_empty() {
        return Ok(exact);
    }

    // Try case variations
    for variant in case_variations(name).into_iter().skip(1) {
        debug!(original = name, variant = %variant, "trying case variation");
        let results = client.workspace_symbol(&variant).await?;
        let exact = filter_exact_matches(&results, &variant);
        if !exact.is_empty() {
            return Ok(exact);
        }
    }

    // Fuzzy: use original query results and find best matches by similarity
    let results = client.workspace_symbol(name).await?;
    if !results.is_empty() {
        let mut scored: Vec<(f64, SymbolInformation)> = results
            .into_iter()
            .map(|s| {
                let score = jaro_winkler(&s.name, name);
                (score, s)
            })
            .collect();
        scored.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));

        let good: Vec<SymbolInformation> = scored
            .into_iter()
            .filter(|(score, _)| *score > 0.8)
            .map(|(_, s)| s)
            .collect();

        if !good.is_empty() {
            return Ok(good);
        }
    }

    Ok(vec![])
}

// ── workspace/symbol-based parent/child matching ─────────────────────────────

/// Attempt to match `child` symbols from workspace/symbol whose `container_name`
/// matches `parent`.
///
/// `fuzzy_parent` – use Jaro-Winkler similarity (> 0.8) for the container check.
/// `fuzzy_child`  – use Jaro-Winkler similarity (> 0.8) for the child name check.
async fn try_parent_child_workspace(
    client: &Arc<LspClient>,
    parent: &str,
    child: &str,
    fuzzy_parent: bool,
    fuzzy_child: bool,
) -> Result<Option<Vec<SymbolInformation>>, crate::lsp::client::LspClientError> {
    let results = client.workspace_symbol(child).await?;

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
        Ok(None)
    } else {
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
}

// ── document/symbol-based parent/child matching ───────────────────────────────

/// Find the parent symbol first (via workspace/symbol), then search for `child`
/// inside the parent's source file using textDocument/documentSymbol.
///
/// This handles the case where workspace/symbol doesn't index the method at all
/// (e.g. short method names that are substrings of type names confuse the query).
async fn try_parent_child_via_document(
    client: &Arc<LspClient>,
    parent: &str,
    child: &str,
    fuzzy_parent: bool,
    fuzzy_child: bool,
) -> Result<Option<Vec<SymbolInformation>>, crate::lsp::client::LspClientError> {
    // Find parent via workspace/symbol
    let parent_results = client.workspace_symbol(parent).await?;
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
        // Open file so the LSP tracks it
        if let Some(path) = uri_to_path(uri) {
            let _ = client.open_file(&path).await;
        }

        let doc_symbols = match client.document_symbol(uri).await {
            Ok(s) => s,
            Err(_) => continue,
        };

        let found = collect_doc_symbol_matches(&doc_symbols, uri, child, fuzzy_child);
        all_matches.extend(found);
    }

    if all_matches.is_empty() {
        Ok(None)
    } else {
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
}

/// Walk a `DocumentSymbolResponse` and collect all symbols whose name matches
/// `child`, returning them as `SymbolInformation` with the given `uri`.
fn collect_doc_symbol_matches(
    response: &DocumentSymbolResponse,
    uri: &Uri,
    child: &str,
    fuzzy: bool,
) -> Vec<SymbolInformation> {
    let mut out = Vec::new();
    match response {
        DocumentSymbolResponse::Flat(symbols) => {
            for sym in symbols {
                if child_name_matches(&sym.name, child, fuzzy) {
                    out.push(sym.clone());
                }
            }
        }
        DocumentSymbolResponse::Nested(symbols) => {
            collect_nested_doc_matches(symbols, uri, child, fuzzy, None, &mut out);
        }
    }
    out
}

fn collect_nested_doc_matches(
    symbols: &[lsp_types::DocumentSymbol],
    uri: &Uri,
    child: &str,
    fuzzy: bool,
    container: Option<&str>,
    out: &mut Vec<SymbolInformation>,
) {
    for sym in symbols {
        if child_name_matches(&sym.name, child, fuzzy) {
            #[allow(deprecated)]
            out.push(SymbolInformation {
                name: sym.name.clone(),
                kind: sym.kind,
                tags: sym.tags.clone(),
                deprecated: sym.deprecated.map(|_| false),
                location: Location {
                    uri: uri.clone(),
                    range: sym.selection_range,
                },
                container_name: container.map(str::to_string),
            });
        }
        if let Some(children) = &sym.children {
            collect_nested_doc_matches(children, uri, child, fuzzy, Some(&sym.name), out);
        }
    }
}

// ── shared helpers ────────────────────────────────────────────────────────────

/// Check whether a symbol name matches the query `expected` (exact or fuzzy).
fn child_name_matches(name: &str, expected: &str, fuzzy: bool) -> bool {
    if fuzzy {
        jaro_winkler(name, expected) > 0.8
    } else {
        name == expected
            || name.ends_with(&format!(".{expected}"))
            || name.ends_with(&format!("::{expected}"))
    }
}

/// Check whether a container name (from workspace/symbol) matches `parent`.
fn container_matches(container: &str, parent: &str, fuzzy: bool) -> bool {
    // Strip generic type parameters: "RelayChannel<W, R>" → "RelayChannel"
    let base = container.split('<').next().unwrap_or(container).trim();
    // Strip qualified path: "foo::bar::RelayChannel" → "RelayChannel"
    let base = base.rsplit("::").next().unwrap_or(base);
    let base = base.rsplit('.').next().unwrap_or(base).trim();

    // Also handle "impl RelayChannel<W, R>" style container names from rust-analyzer
    let strip_impl = container
        .strip_prefix("impl ")
        .unwrap_or(container)
        .split('<')
        .next()
        .unwrap_or(container)
        .trim()
        .rsplit("::")
        .next()
        .unwrap_or(container)
        .rsplit('.')
        .next()
        .unwrap_or(container)
        .trim();

    if fuzzy {
        jaro_winkler(base, parent) > 0.8
            || jaro_winkler(strip_impl, parent) > 0.8
            || jaro_winkler(container, parent) > 0.8
    } else {
        base == parent
            || strip_impl == parent
            || container == parent
            || container.ends_with(&format!("::{parent}"))
            || container.ends_with(&format!(".{parent}"))
    }
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

/// Find similar symbols within a document (fallback when workspace/symbol fails).
#[allow(dead_code)]
pub async fn find_similar_in_document(
    client: &Arc<LspClient>,
    uri: &Uri,
    name: &str,
    threshold: f64,
) -> Result<Vec<(String, f64)>, crate::lsp::client::LspClientError> {
    let doc_symbols = client.document_symbol(uri).await?;
    let mut matches = Vec::new();

    match doc_symbols {
        DocumentSymbolResponse::Flat(symbols) => {
            for sym in symbols {
                let score = jaro_winkler(&sym.name, name);
                if score >= threshold {
                    matches.push((sym.name.clone(), score));
                }
            }
        }
        DocumentSymbolResponse::Nested(symbols) => {
            collect_nested_matches(&symbols, name, threshold, &mut matches);
        }
    }

    matches.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
    Ok(matches)
}

#[allow(dead_code)]
fn collect_nested_matches(
    symbols: &[lsp_types::DocumentSymbol],
    name: &str,
    threshold: f64,
    matches: &mut Vec<(String, f64)>,
) {
    for sym in symbols {
        let score = jaro_winkler(&sym.name, name);
        if score >= threshold {
            matches.push((sym.name.clone(), score));
        }
        if let Some(children) = &sym.children {
            collect_nested_matches(children, name, threshold, matches);
        }
    }
}
