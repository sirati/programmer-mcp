//! Symbol cache with fuzzy search indexing.
//!
//! Wraps LSP `workspace/symbol` calls with a local cache that:
//! - Avoids redundant LSP round-trips for repeated queries
//! - Maintains a merged symbol index for fast local fuzzy matching
//! - Invalidates entries when source files change

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use lsp_types::SymbolInformation;
use nucleo_matcher::pattern::{AtomKind, CaseMatching, Normalization, Pattern};
use nucleo_matcher::{Config, Matcher, Utf32Str};
use tokio::sync::RwLock;
use tracing::{debug, trace};

use crate::lsp::client::{LspClient, LspClientError};

/// How long cached workspace_symbol results stay valid.
const CACHE_TTL: Duration = Duration::from_secs(60);

/// A cached workspace_symbol query result.
struct CachedQuery {
    symbols: Vec<SymbolInformation>,
    fetched_at: Instant,
}

/// Entry in the merged symbol index for fuzzy matching.
#[derive(Clone)]
struct IndexEntry {
    /// The symbol name (used for fuzzy matching).
    name: String,
    /// The full SymbolInformation from the LSP.
    symbol: SymbolInformation,
}

/// Per-client symbol cache.
pub struct SymbolCache {
    /// Query string → cached results.
    query_cache: RwLock<HashMap<String, CachedQuery>>,
    /// Merged index of all symbols seen, keyed by symbol name for dedup.
    /// Key is (name, uri, line) to avoid duplicates.
    index: RwLock<HashMap<(String, String, u32), IndexEntry>>,
}

impl SymbolCache {
    pub fn new() -> Self {
        Self {
            query_cache: RwLock::new(HashMap::new()),
            index: RwLock::new(HashMap::new()),
        }
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
        self.add_to_index(&symbols).await;

        Ok(symbols)
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
        // Remove all query cache entries (conservative: any file change could affect any query)
        // A more precise approach would track which queries returned results from this file,
        // but the simple approach is fine since the TTL is short.
        {
            let mut cache = self.query_cache.write().await;
            cache.clear();
        }

        // Remove index entries from this file
        {
            let mut index = self.index.write().await;
            index.retain(|(_name, uri, _line), _| uri != file_uri);
        }

        trace!(file_uri, "symbol cache invalidated for file");
    }

    /// Seed the cache by querying for common prefixes.
    /// Call this after LSP initialization to pre-populate the index.
    pub async fn seed(&self, client: &Arc<LspClient>) {
        // Query with empty string to get all symbols (many LSPs support this)
        if let Ok(symbols) = client.workspace_symbol("").await {
            debug!(count = symbols.len(), "seeded symbol cache");
            self.add_to_index(&symbols).await;
            let mut cache = self.query_cache.write().await;
            cache.insert(
                String::new(),
                CachedQuery {
                    symbols,
                    fetched_at: Instant::now(),
                },
            );
        }
    }

    /// Add symbols to the merged index.
    async fn add_to_index(&self, symbols: &[SymbolInformation]) {
        let mut index = self.index.write().await;
        for sym in symbols {
            let uri = sym.location.uri.as_str().to_string();
            let line = sym.location.range.start.line;
            let key = (sym.name.clone(), uri, line);
            if !index.contains_key(&key) {
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

    /// Get the number of cached queries and indexed symbols.
    pub async fn stats(&self) -> (usize, usize) {
        let queries = self.query_cache.read().await.len();
        let symbols = self.index.read().await.len();
        (queries, symbols)
    }
}
