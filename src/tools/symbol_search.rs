use std::sync::Arc;

use heck::{ToLowerCamelCase, ToPascalCase, ToShoutySnakeCase, ToSnakeCase};
use lsp_types::{DocumentSymbolResponse, SymbolInformation, Uri};
use strsim::jaro_winkler;
use tracing::debug;

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
pub async fn find_symbol_with_fallback(
    client: &Arc<LspClient>,
    name: &str,
) -> Result<Vec<SymbolInformation>, crate::lsp::client::LspClientError> {
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

        // Return symbols with score > 0.8
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

/// Filter symbol results to exact name matches, handling qualified names.
fn filter_exact_matches(symbols: &[SymbolInformation], name: &str) -> Vec<SymbolInformation> {
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
