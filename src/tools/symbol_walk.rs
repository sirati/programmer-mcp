//! Directory-walk fallback for symbol resolution.
//!
//! When `workspace/symbol` fails to locate a symbol, this module walks upward
//! from a starting directory, scanning source files via `documentSymbol` at
//! each level until the workspace root is reached.

use std::path::Path;
use std::sync::Arc;

use lsp_types::SymbolInformation;
use tracing::debug;

use super::dsl::ops::SOURCE_EXTENSIONS;
use super::formatting::path_to_uri;
use super::symbol_match::collect_doc_symbol_matches;
use crate::lsp::client::LspClient;

/// Walk upward from `start_dir` to workspace root, scanning document symbols
/// in source files at each directory level. Returns the first match found.
pub async fn try_directory_walk(
    client: &Arc<LspClient>,
    name: &str,
    start_dir: &str,
) -> Result<Option<Vec<SymbolInformation>>, crate::lsp::client::LspClientError> {
    let workspace_root = std::env::current_dir().unwrap_or_default();
    let mut dir = workspace_root.join(start_dir);

    // Handle dotted names — search for the leaf
    let search_name = name.rsplit('.').next().unwrap_or(name);

    loop {
        if let Some(found) = scan_dir_for_symbol(client, search_name, &dir).await? {
            let rel = dir
                .strip_prefix(&workspace_root)
                .unwrap_or(&dir)
                .display()
                .to_string();
            debug!(
                symbol = name,
                resolved_at = %rel,
                "found symbol via directory walk"
            );
            return Ok(Some(found));
        }

        // Move up one level, stop at workspace root
        if dir == workspace_root || !dir.starts_with(&workspace_root) {
            break;
        }
        match dir.parent() {
            Some(parent) if parent >= workspace_root.as_path() => dir = parent.to_path_buf(),
            _ => break,
        }
    }

    Ok(None)
}

/// Scan all source files in `dir` for a symbol matching `name`.
async fn scan_dir_for_symbol(
    client: &Arc<LspClient>,
    name: &str,
    dir: &Path,
) -> Result<Option<Vec<SymbolInformation>>, crate::lsp::client::LspClientError> {
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return Ok(None),
    };

    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
        if !SOURCE_EXTENSIONS.contains(&ext) {
            continue;
        }

        let path_str = path.display().to_string();
        let uri = match path_to_uri(&path_str) {
            Ok(u) => u,
            Err(_) => continue,
        };

        client.open_file(&path_str).await.ok();
        let doc_symbols = match client.document_symbol(&uri).await {
            Ok(s) => s,
            Err(_) => continue,
        };

        // Search for exact match first, then fuzzy
        let exact = collect_doc_symbol_matches(&doc_symbols, &uri, name, false);
        if !exact.is_empty() {
            return Ok(Some(exact));
        }
        let fuzzy = collect_doc_symbol_matches(&doc_symbols, &uri, name, true);
        if !fuzzy.is_empty() {
            return Ok(Some(fuzzy));
        }
    }

    Ok(None)
}
