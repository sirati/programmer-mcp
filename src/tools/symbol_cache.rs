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
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use lsp_types::{DocumentSymbolResponse, Location, SymbolInformation, Uri};
use nucleo_matcher::pattern::{AtomKind, CaseMatching, Normalization, Pattern};
use nucleo_matcher::{Config, Matcher, Utf32Str};
use tokio::sync::RwLock;
use tracing::{debug, trace};

use super::formatting::path_to_uri;
use super::SOURCE_EXTS;
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
    /// Merged index of all symbols seen, keyed by (name, uri, line) for dedup.
    index: RwLock<HashMap<(String, String, u32), IndexEntry>>,
    /// Name → list of index keys, for fast exact lookup.
    name_index: RwLock<HashMap<String, Vec<(String, String, u32)>>>,
}

impl SymbolCache {
    pub fn new() -> Self {
        Self {
            query_cache: RwLock::new(HashMap::new()),
            index: RwLock::new(HashMap::new()),
            name_index: RwLock::new(HashMap::new()),
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
            // Update name_index
            let mut name_idx = self.name_index.write().await;
            for key in &removed {
                if let Some(keys) = name_idx.get_mut(&key.0) {
                    keys.retain(|k| k != key);
                    if keys.is_empty() {
                        name_idx.remove(&key.0);
                    }
                }
            }
        }

        trace!(file_uri, "symbol cache invalidated for file");
    }

    /// Seed via workspace/symbol queries (for LSPs that support it).
    pub async fn seed_workspace_symbols(&self, client: &Arc<LspClient>) {
        let mut total = 0;
        for query in [
            "", "a", "b", "c", "d", "e", "f", "g", "h", "i", "m", "n", "o", "p", "r", "s", "t",
            "u", "w",
        ] {
            match client.workspace_symbol(query).await {
                Ok(symbols) if !symbols.is_empty() => {
                    total += symbols.len();
                    let mut cache = self.query_cache.write().await;
                    cache.insert(
                        query.to_string(),
                        CachedQuery {
                            symbols: symbols.clone(),
                            fetched_at: Instant::now(),
                        },
                    );
                    drop(cache);
                    self.add_to_index(&symbols).await;
                    if query.is_empty() {
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
    /// Used for LSPs that don't support workspace/symbol (e.g. basedpyright).
    pub async fn seed_from_documents(&self, client: &Arc<LspClient>, workspace: &Path) {
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
            self.add_to_index(&flat).await;
        }
        debug!(lsp = %lang, total, files = files.len(), "seeded symbol cache (documentSymbol)");
    }

    /// Seed the cache — picks the right strategy based on LSP capabilities.
    /// If workspace/symbol is advertised but returns nothing, falls back to
    /// document symbol scanning.
    pub async fn seed(&self, client: &Arc<LspClient>, workspace: &Path) {
        // Wait for the LSP to finish initial indexing.
        tokio::time::sleep(Duration::from_secs(5)).await;

        if client.has_workspace_symbol() {
            self.seed_workspace_symbols(client).await;
            // If workspace/symbol yielded nothing, the LSP may advertise the
            // capability but not actually return results (e.g. basedpyright).
            let (_, indexed) = self.stats().await;
            if indexed == 0 {
                debug!(lsp = %client.language(), "workspace/symbol returned nothing, falling back to document scan");
                self.seed_from_documents(client, workspace).await;
            }
        } else {
            self.seed_from_documents(client, workspace).await;
        }
    }

    /// Add symbols to the merged index.
    async fn add_to_index(&self, symbols: &[SymbolInformation]) {
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
        self.add_to_index(&flat).await;
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

// ── Helpers ──────────────────────────────────────────────────────────────────

/// Flatten a `DocumentSymbolResponse` into a list of `SymbolInformation`,
/// preserving container names from nesting.
fn flatten_doc_symbols(response: &DocumentSymbolResponse, uri: &Uri) -> Vec<SymbolInformation> {
    let mut out = Vec::new();
    match response {
        DocumentSymbolResponse::Flat(symbols) => {
            out.extend(symbols.iter().cloned());
        }
        DocumentSymbolResponse::Nested(symbols) => {
            flatten_nested(symbols, uri, None, &mut out);
        }
    }
    out
}

fn flatten_nested(
    symbols: &[lsp_types::DocumentSymbol],
    uri: &Uri,
    container: Option<&str>,
    out: &mut Vec<SymbolInformation>,
) {
    for sym in symbols {
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
        if let Some(children) = &sym.children {
            flatten_nested(children, uri, Some(&sym.name), out);
        }
    }
}

/// Collect source files under `workspace` that match the given language.
fn collect_language_files(workspace: &Path, language: &str) -> Vec<PathBuf> {
    let exts = language_extensions(language);
    let mut files = Vec::new();
    collect_files_recursive(workspace, &exts, &mut files);
    files
}

fn collect_files_recursive(dir: &Path, exts: &[&str], out: &mut Vec<PathBuf>) {
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return,
    };

    for entry in entries.flatten() {
        let path = entry.path();
        let fname = entry.file_name();
        let fname_str = fname.to_string_lossy();

        if fname_str.starts_with('.')
            || fname_str == "target"
            || fname_str == "node_modules"
            || fname_str == "__pycache__"
            || fname_str == "venv"
            || fname_str == ".venv"
        {
            continue;
        }

        if path.is_dir() {
            collect_files_recursive(&path, exts, out);
        } else if path.is_file() {
            let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
            if exts.contains(&ext) {
                out.push(path);
            }
        }
    }
}

/// Map a language identifier to its file extensions.
fn language_extensions(language: &str) -> Vec<&'static str> {
    match language {
        "python" => vec!["py"],
        "rust" => vec!["rs"],
        "go" => vec!["go"],
        "javascript" => vec!["js", "jsx"],
        "typescript" => vec!["ts", "tsx"],
        "nix" => vec!["nix"],
        "c" | "cpp" => vec!["c", "cc", "cpp", "h", "hpp"],
        "java" => vec!["java"],
        "ruby" => vec!["rb"],
        "lua" => vec!["lua"],
        _ => SOURCE_EXTS.to_vec(),
    }
}
