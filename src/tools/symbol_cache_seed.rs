//! Symbol cache seeding: populates the index from LSP servers.
//!
//! Extracted from `symbol_cache.rs` for size compliance.

use std::path::Path;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::{Duration, Instant};

use lsp_types::SymbolInformation;
use tracing::{debug, trace};

use super::doc_index::{collect_language_files, flatten_doc_symbols};
use super::formatting::path_to_uri;
use super::symbol_cache::{CachedQuery, SymbolCache};
use super::symbol_cache_persist;
use crate::lsp::client::LspClient;

/// Seed the cache using all strategies for maximum coverage.
///
/// 1. Try loading from disk cache (only re-index changed files)
/// 2. workspace/symbol (fast, gives public symbols across the workspace)
/// 3. documentSymbol scan (complete — captures private/nested symbols)
///
/// After seeding, the index is saved to disk.
pub async fn seed(cache: &SymbolCache, client: &Arc<LspClient>, workspace: &Path) {
    cache.seeding.store(true, Ordering::Relaxed);

    let lang = client.language().to_string();

    // Phase 0: try loading from disk cache.
    let (had_cache, stale_uris) =
        if let Some((cached_symbols, stale)) = symbol_cache_persist::load(workspace, &lang) {
            debug!(
                language = %lang,
                cached = cached_symbols.len(),
                stale = stale.len(),
                "loaded symbol cache from disk"
            );
            cache.add_symbols(&cached_symbols).await;
            (true, stale)
        } else {
            (false, vec![])
        };

    // Wait for the LSP to finish initial indexing.
    tokio::time::sleep(Duration::from_secs(5)).await;

    // Phase 1: workspace/symbol for quick broad coverage.
    if client.has_workspace_symbol() {
        seed_workspace_symbols(cache, client).await;
    }

    // Phase 2: re-index stale files or do full scan if no cache.
    if had_cache {
        if !stale_uris.is_empty() {
            reindex_files(cache, client, &stale_uris).await;
        }
    } else {
        seed_from_documents(cache, client, workspace).await;
    }

    // Phase 3: save updated index to disk.
    let index = cache.index.read().await;
    let symbols: Vec<SymbolInformation> = index.values().map(|e| e.symbol.clone()).collect();
    drop(index);
    symbol_cache_persist::save(workspace, &lang, &symbols);

    cache.seeding.store(false, Ordering::Relaxed);
}

/// Seed via workspace/symbol queries.
///
/// Strategy: try empty query first. If the LSP returns a large result
/// (500+), assume it supports full listing and stop. Otherwise, probe
/// with single-letter queries to gather more coverage.
async fn seed_workspace_symbols(cache: &SymbolCache, client: &Arc<LspClient>) {
    let queries: &[&str] = &[
        "", "a", "b", "c", "d", "e", "f", "g", "h", "i", "j", "k", "l", "m", "n", "o", "p", "r",
        "s", "t", "u", "v", "w", "x",
    ];
    let mut total = 0;
    for query in queries {
        match client.workspace_symbol(query).await {
            Ok(symbols) if !symbols.is_empty() => {
                total += symbols.len();
                let mut qcache = cache.query_cache.write().await;
                qcache.insert(
                    query.to_string(),
                    CachedQuery {
                        symbols: symbols.clone(),
                        fetched_at: Instant::now(),
                    },
                );
                drop(qcache);
                cache.add_symbols(&symbols).await;
                if query.is_empty() && symbols.len() >= 500 {
                    break;
                }
            }
            Ok(_) => {}
            Err(_) => break,
        }
    }
    debug!(lsp = %client.language(), total, "seeded symbol cache (workspace/symbol)");
}

/// Seed by walking source files and indexing their document symbols.
async fn seed_from_documents(cache: &SymbolCache, client: &Arc<LspClient>, workspace: &Path) {
    let lang = client.language();
    let files = collect_language_files(workspace, lang);
    debug!(lsp = %lang, file_count = files.len(), "scanning files for document symbols");

    let mut total = 0;
    for path in &files {
        let path_str = path.display().to_string();
        let uri = match path_to_uri(&path_str) {
            Ok(u) => u,
            Err(_) => continue,
        };

        if let Err(e) = client.open_file(&path_str).await {
            trace!(file = %path_str, "failed to open for indexing: {e}");
            continue;
        }

        let doc_symbols = match client.document_symbol(&uri).await {
            Ok(s) => s,
            Err(e) => {
                trace!(file = %path_str, "documentSymbol failed: {e}");
                continue;
            }
        };

        let flat = flatten_doc_symbols(&doc_symbols, &uri);
        total += flat.len();
        cache.add_symbols(&flat).await;
    }
    debug!(lsp = %lang, total, files = files.len(), "seeded symbol cache (documentSymbol)");
}

/// Re-index specific files (used when cache has stale entries).
async fn reindex_files(cache: &SymbolCache, client: &Arc<LspClient>, file_uris: &[String]) {
    let mut total = 0;
    for uri_str in file_uris {
        let uri: lsp_types::Uri = match uri_str.parse() {
            Ok(u) => u,
            Err(_) => continue,
        };
        let path_str = uri_str.strip_prefix("file://").unwrap_or(uri_str);
        if !Path::new(path_str).exists() {
            continue;
        }
        if let Err(e) = client.open_file(path_str).await {
            trace!(file = %path_str, "failed to open for reindexing: {e}");
            continue;
        }
        match client.document_symbol(&uri).await {
            Ok(doc_symbols) => {
                let flat = flatten_doc_symbols(&doc_symbols, &uri);
                total += flat.len();
                cache.add_symbols(&flat).await;
            }
            Err(e) => {
                trace!(file = %path_str, "documentSymbol failed during reindex: {e}");
            }
        }
    }
    debug!(files = file_uris.len(), total, "reindexed stale files");
}
