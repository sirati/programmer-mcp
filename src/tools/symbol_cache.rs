//! Symbol cache with fuzzy search indexing.
//!
//! Wraps LSP `workspace/symbol` calls with a local cache that:
//! - Avoids redundant LSP round-trips for repeated queries
//! - Maintains a merged symbol index for fast local fuzzy matching
//! - Invalidates entries when source files change
//!
//! For LSPs without `workspace/symbol` support, the cache can be seeded
//! from `documentSymbol` responses by walking source files.

use std::collections::HashMap;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use lsp_types::{SymbolInformation, Uri};
use nucleo_matcher::pattern::{AtomKind, CaseMatching, Normalization, Pattern};
use nucleo_matcher::{Config, Matcher, Utf32Str};
use tokio::sync::RwLock;
use tracing::{debug, trace};

use super::doc_index::flatten_doc_symbols;

use crate::lsp::client::{LspClient, LspClientError};

/// How long cached workspace_symbol results stay valid.
const CACHE_TTL: Duration = Duration::from_secs(60);

/// A cached workspace_symbol query result.
pub(super) struct CachedQuery {
    pub symbols: Vec<SymbolInformation>,
    pub fetched_at: Instant,
}

/// Entry in the merged symbol index for fuzzy matching.
#[derive(Clone)]
pub(super) struct IndexEntry {
    /// The symbol name (used for fuzzy matching).
    pub name: String,
    /// The full SymbolInformation from the LSP.
    pub symbol: SymbolInformation,
}

/// Per-client symbol cache.
pub struct SymbolCache {
    /// Query string → cached results.
    pub(super) query_cache: RwLock<HashMap<String, CachedQuery>>,
    /// Merged index of all symbols seen, keyed by (name, uri, line) for dedup.
    pub(super) index: RwLock<HashMap<(String, String, u32), IndexEntry>>,
    /// Name → list of index keys, for fast exact lookup.
    pub(super) name_index: RwLock<HashMap<String, Vec<(String, String, u32)>>>,
    /// True while initial seeding is in progress.
    pub(super) seeding: AtomicBool,
}

impl SymbolCache {
    pub fn new() -> Self {
        Self {
            query_cache: RwLock::new(HashMap::new()),
            index: RwLock::new(HashMap::new()),
            name_index: RwLock::new(HashMap::new()),
            seeding: AtomicBool::new(false),
        }
    }

    /// Returns true if initial seeding is still in progress.
    pub fn is_seeding(&self) -> bool {
        self.seeding.load(Ordering::Relaxed)
    }

    /// Query workspace symbols, using cache when available.
    pub async fn workspace_symbol(
        &self,
        client: &Arc<LspClient>,
        query: &str,
    ) -> Result<Vec<SymbolInformation>, LspClientError> {
        // Check cache first
        {
            let cache = self.query_cache.read().await;
            if let Some(entry) = cache.get(query) {
                if entry.fetched_at.elapsed() < CACHE_TTL {
                    debug!(query, "symbol cache hit");
                    return Ok(entry.symbols.clone());
                }
            }
        }

        // Cache miss or expired — query LSP
        let symbols = client.workspace_symbol(query).await?;

        // Update query cache
        {
            let mut cache = self.query_cache.write().await;
            cache.insert(
                query.to_string(),
                CachedQuery {
                    symbols: symbols.clone(),
                    fetched_at: Instant::now(),
                },
            );
        }

        // Merge into index
        self.add_symbols(&symbols).await;

        Ok(symbols)
    }

    /// Exact name lookup from the index. Returns all symbols with this exact name.
    pub async fn exact_search(&self, name: &str) -> Vec<SymbolInformation> {
        let name_idx = self.name_index.read().await;
        let Some(keys) = name_idx.get(name) else {
            return vec![];
        };
        let index = self.index.read().await;
        keys.iter()
            .filter_map(|k| index.get(k).map(|e| e.symbol.clone()))
            .collect()
    }

    /// Fuzzy search the local symbol index without hitting the LSP.
    /// Returns symbols sorted by match score (best first).
    pub async fn fuzzy_search(&self, query: &str, limit: usize) -> Vec<SymbolInformation> {
        let index = self.index.read().await;
        if index.is_empty() {
            return vec![];
        }

        let mut matcher = Matcher::new(Config::DEFAULT);
        let pattern = Pattern::new(
            query,
            CaseMatching::Ignore,
            Normalization::Smart,
            AtomKind::Fuzzy,
        );

        let mut scored: Vec<(u32, SymbolInformation)> = Vec::new();

        for entry in index.values() {
            let mut buf = Vec::new();
            let haystack = Utf32Str::new(&entry.name, &mut buf);
            if let Some(score) = pattern.score(haystack, &mut matcher) {
                scored.push((score, entry.symbol.clone()));
            }
        }

        // Sort by score descending
        scored.sort_by(|a, b| b.0.cmp(&a.0));
        scored.truncate(limit);
        scored.into_iter().map(|(_, s)| s).collect()
    }

    /// Invalidate cache entries related to a changed file.
    pub async fn invalidate_file(&self, file_uri: &str) {
        {
            let mut cache = self.query_cache.write().await;
            cache.clear();
        }

        // Remove index entries from this file and update name_index.
        {
            let mut index = self.index.write().await;
            let removed: Vec<(String, String, u32)> = index
                .keys()
                .filter(|(_, uri, _)| uri == file_uri)
                .cloned()
                .collect();
            for key in &removed {
                index.remove(key);
            }
            // Update name_index (both full name and bare name entries)
            let mut name_idx = self.name_index.write().await;
            for key in &removed {
                for name in [key.0.as_str(), extract_bare_name(&key.0)] {
                    if let Some(keys) = name_idx.get_mut(name) {
                        keys.retain(|k| k != key);
                        if keys.is_empty() {
                            name_idx.remove(name);
                        }
                    }
                }
            }
        }

        trace!(file_uri, "symbol cache invalidated for file");
    }

    /// Seed the cache using all strategies (disk cache, workspace/symbol,
    /// documentSymbol scan). See `symbol_cache_seed` module for implementation.
    pub async fn seed(&self, client: &Arc<LspClient>, workspace: &Path) {
        super::symbol_cache_seed::seed(self, client, workspace).await;
    }

    /// Add symbols to the merged index (public for directory-walk indexing).
    pub async fn add_symbols(&self, symbols: &[SymbolInformation]) {
        let mut index = self.index.write().await;
        let mut name_idx = self.name_index.write().await;
        for sym in symbols {
            let uri = sym.location.uri.as_str().to_string();
            let line = sym.location.range.start.line;
            let key = (sym.name.clone(), uri, line);
            if !index.contains_key(&key) {
                name_idx
                    .entry(sym.name.clone())
                    .or_default()
                    .push(key.clone());
                // Also index by bare name for Go-style (*Type).Method etc.
                let bare = extract_bare_name(&sym.name);
                if bare != sym.name {
                    name_idx
                        .entry(bare.to_string())
                        .or_default()
                        .push(key.clone());
                }
                index.insert(
                    key,
                    IndexEntry {
                        name: sym.name.clone(),
                        symbol: sym.clone(),
                    },
                );
            }
        }
    }

    /// Index document symbols from a single file (used on-demand during resolution).
    pub async fn index_file(
        &self,
        client: &Arc<LspClient>,
        uri: &Uri,
    ) -> Result<(), LspClientError> {
        let path_str = uri.as_str().strip_prefix("file://").unwrap_or(uri.as_str());
        client.open_file(path_str).await.ok();
        let doc_symbols = client.document_symbol(uri).await?;
        let flat = flatten_doc_symbols(&doc_symbols, uri);
        self.add_symbols(&flat).await;
        Ok(())
    }

    /// Get the number of cached queries and indexed symbols.
    #[allow(dead_code)]
    pub async fn stats(&self) -> (usize, usize) {
        let queries = self.query_cache.read().await.len();
        let symbols = self.index.read().await.len();
        (queries, symbols)
    }
}

/// Extract the bare name from a qualified symbol name.
/// e.g. `(*Client).Call` → `Call`, `foo::bar` → `bar`, `Foo.bar` → `bar`
fn extract_bare_name(name: &str) -> &str {
    // Go-style: (*T).Method or (T).Method
    if let Some(pos) = name.rfind(").") {
        return &name[pos + 2..];
    }
    if let Some(pos) = name.rfind("::") {
        return &name[pos + 2..];
    }
    if let Some(pos) = name.rfind('.') {
        return &name[pos + 1..];
    }
    name
}
