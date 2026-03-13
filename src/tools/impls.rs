use std::fmt::Write;
use std::sync::Arc;

use lsp_types::DocumentSymbolResponse;

use super::formatting::uri_to_path;
use super::symbol_info::not_found_msg;
use super::symbol_search::find_symbol_with_fallback;
use crate::lsp::client::{LspClient, LspClientError};

/// Find all impl blocks for a type by searching references and filtering for impl blocks.
pub async fn find_impls(
    client: &Arc<LspClient>,
    type_name: &str,
    search_dir: Option<&str>,
) -> Result<String, LspClientError> {
    let symbols = find_symbol_with_fallback(client, type_name, search_dir).await?;
    if symbols.is_empty() {
        return Ok(not_found_msg(client, type_name));
    }

    // Get the file where the type is defined
    let sym = &symbols[0];
    let uri = &sym.location.uri;
    let path = uri_to_path(uri).unwrap_or_else(|| uri.as_str().to_string());
    client.open_file(&path).await?;

    // Get all references to this type
    let refs = client
        .references(uri, sym.location.range.start, true)
        .await?;
    let Some(locations) = refs else {
        return Ok(format!("No references found for {type_name}"));
    };

    // For each reference location, check if it's inside an impl block
    let mut impl_blocks = Vec::new();
    let mut seen_files = std::collections::HashSet::new();

    for loc in &locations {
        let ref_path = uri_to_path(&loc.uri).unwrap_or_else(|| loc.uri.as_str().to_string());
        if !seen_files.contains(&ref_path) {
            seen_files.insert(ref_path.clone());
            client.open_file(&ref_path).await.ok();
        }

        // Get document symbols for this file to find impl blocks containing this reference
        let doc_symbols = client.document_symbol(&loc.uri).await.ok();
        if let Some(doc_symbols) = doc_symbols {
            if let Some(impl_info) = find_impl_at(&doc_symbols, loc.range.start.line, type_name) {
                if !impl_blocks.contains(&impl_info) {
                    impl_blocks.push(impl_info);
                }
            }
        }
    }

    if impl_blocks.is_empty() {
        return Ok(format!("No impl blocks found for {type_name}"));
    }

    let mut out = String::new();
    for info in &impl_blocks {
        writeln!(out, "- {}", info).ok();
    }
    Ok(out.trim_end().to_string())
}

/// Find the impl block (if any) that contains the given line and is an impl of `type_name`.
fn find_impl_at(response: &DocumentSymbolResponse, line: u32, type_name: &str) -> Option<String> {
    match response {
        DocumentSymbolResponse::Nested(symbols) => {
            for sym in symbols {
                if sym.range.start.line <= line && sym.range.end.line >= line {
                    let name = &sym.name;
                    if name.starts_with("impl") && impl_matches_type(name, type_name) {
                        let detail = sym.detail.as_deref().unwrap_or("");
                        if detail.is_empty() {
                            return Some(name.clone());
                        }
                        return Some(format!("{name} — {detail}"));
                    }
                }
            }
            None
        }
        DocumentSymbolResponse::Flat(symbols) => {
            for sym in symbols {
                if sym.location.range.start.line <= line && sym.location.range.end.line >= line {
                    if sym.name.starts_with("impl") && impl_matches_type(&sym.name, type_name) {
                        return Some(sym.name.clone());
                    }
                }
            }
            None
        }
    }
}

/// Check if an impl block name like "impl Foo" or "impl Trait for Foo" matches the type.
fn impl_matches_type(impl_name: &str, type_name: &str) -> bool {
    // Patterns: "impl Type", "impl Type<...>", "impl Trait for Type", "impl Trait for Type<...>"
    let name = impl_name.trim_start_matches("impl ");
    // "Trait for Type" case
    if let Some(after_for) = name.split(" for ").nth(1) {
        let base = after_for.split('<').next().unwrap_or(after_for).trim();
        return base == type_name;
    }
    // "impl Type" case
    let base = name.split('<').next().unwrap_or(name).trim();
    base == type_name
}
