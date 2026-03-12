//! Directory-walk fallback for symbol resolution.
//!
//! When `workspace/symbol` fails to locate a symbol, this module walks upward
//! from a starting directory, scanning source files via `documentSymbol` at
//! each level (including subdirectories) until the workspace root is reached.

use std::path::Path;
use std::sync::Arc;

use lsp_types::SymbolInformation;
use tracing::debug;

use super::doc_index::flatten_doc_symbols;
use super::formatting::path_to_uri;
use super::symbol_match::collect_doc_symbol_matches;
use super::SOURCE_EXTS;
use crate::lsp::client::LspClient;

/// Walk upward from `start_dir` to workspace root, scanning document symbols
/// in source files at each directory level (recursively). Returns the first match found.
pub async fn try_directory_walk(
    client: &Arc<LspClient>,
    name: &str,
    start_dir: &str,
) -> Result<Option<Vec<SymbolInformation>>, crate::lsp::client::LspClientError> {
    let workspace_root = std::env::current_dir().unwrap_or_default();
    let mut dir = workspace_root.join(start_dir);

    // Handle dotted names — search for the leaf
    let search_name = name.rsplit('.').next().unwrap_or(name);

    // First search only the start_dir (no subdirs) for quick hits
    if let Some(found) = scan_dir_files(client, search_name, &dir).await? {
        return Ok(Some(found));
    }

    loop {
        // Search current dir recursively (including all subdirs)
        if let Some(found) = scan_dir_recursive(client, search_name, &dir, &workspace_root).await? {
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

/// Scan source files directly in `dir` (no recursion) for a symbol.
async fn scan_dir_files(
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
        if let Some(found) = check_file_for_symbol(client, name, &path).await? {
            return Ok(Some(found));
        }
    }

    Ok(None)
}

/// Recursively scan source files in `dir` and all subdirs for a symbol.
/// Skips hidden dirs and common non-source directories.
async fn scan_dir_recursive(
    client: &Arc<LspClient>,
    name: &str,
    dir: &Path,
    workspace_root: &Path,
) -> Result<Option<Vec<SymbolInformation>>, crate::lsp::client::LspClientError> {
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return Ok(None),
    };

    let mut files = Vec::new();
    let mut subdirs = Vec::new();

    for entry in entries.flatten() {
        let path = entry.path();
        let name_os = entry.file_name();
        let fname = name_os.to_string_lossy();

        if fname.starts_with('.')
            || fname == "target"
            || fname == "node_modules"
            || fname == "__pycache__"
            || fname == ".git"
        {
            continue;
        }

        if path.is_dir() {
            subdirs.push(path);
        } else if path.is_file() {
            files.push(path);
        }
    }

    // Check files in this directory first
    for path in &files {
        if let Some(found) = check_file_for_symbol(client, name, path).await? {
            return Ok(Some(found));
        }
    }

    // Then recurse into subdirs
    for subdir in &subdirs {
        if let Some(found) =
            Box::pin(scan_dir_recursive(client, name, subdir, workspace_root)).await?
        {
            return Ok(Some(found));
        }
    }

    Ok(None)
}

/// Check a single file for a symbol match (exact first, then fuzzy).
async fn check_file_for_symbol(
    client: &Arc<LspClient>,
    name: &str,
    path: &Path,
) -> Result<Option<Vec<SymbolInformation>>, crate::lsp::client::LspClientError> {
    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
    if !SOURCE_EXTS.contains(&ext) {
        return Ok(None);
    }

    let path_str = path.display().to_string();
    let uri = match path_to_uri(&path_str) {
        Ok(u) => u,
        Err(_) => return Ok(None),
    };

    client.open_file(&path_str).await.ok();
    let doc_symbols = match client.document_symbol(&uri).await {
        Ok(s) => s,
        Err(_) => return Ok(None),
    };

    // Feed all symbols from this file into the index for future lookups.
    let flat = flatten_doc_symbols(&doc_symbols, &uri);
    client.symbol_cache().add_symbols(&flat).await;

    let exact = collect_doc_symbol_matches(&doc_symbols, &uri, name, false);
    if !exact.is_empty() {
        return Ok(Some(exact));
    }
    let fuzzy = collect_doc_symbol_matches(&doc_symbols, &uri, name, true);
    if !fuzzy.is_empty() {
        return Ok(Some(fuzzy));
    }

    Ok(None)
}
