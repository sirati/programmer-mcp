//! Document-symbol matching helpers.
//!
//! Lower-level helpers used by `symbol_search` to match child/container names
//! and collect results from `textDocument/documentSymbol` responses.

use lsp_types::{DocumentSymbolResponse, Location, SymbolInformation, Uri};
use strsim::jaro_winkler;

// ── name matching ─────────────────────────────────────────────────────────────

/// Check whether a symbol name matches the query `expected` (exact or fuzzy).
pub fn child_name_matches(name: &str, expected: &str, fuzzy: bool) -> bool {
    if fuzzy {
        jaro_winkler(name, expected) > 0.8
    } else {
        name == expected
            || name.ends_with(&format!(".{expected}"))
            || name.ends_with(&format!("::{expected}"))
    }
}

/// Check whether a container name (from workspace/symbol) matches `parent`.
pub fn container_matches(container: &str, parent: &str, fuzzy: bool) -> bool {
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

// ── document symbol collection ────────────────────────────────────────────────

/// Walk a `DocumentSymbolResponse` and collect all symbols whose name matches
/// `child`, returning them as `SymbolInformation` with the given `uri`.
pub fn collect_doc_symbol_matches(
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

pub fn collect_nested_doc_matches(
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

// ── similarity scoring ────────────────────────────────────────────────────────

/// Collect symbols from a nested tree whose name is within `threshold`
/// Jaro-Winkler similarity to `name`.
#[allow(dead_code)]
pub fn collect_nested_matches(
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
