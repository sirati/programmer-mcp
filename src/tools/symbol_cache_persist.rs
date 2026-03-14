//! Symbol cache persistence: save/load to `.cache/programmer-mcp/`.
//!
//! Stores the symbol index per language with file modification timestamps
//! so that on restart only changed files need re-indexing.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use lsp_types::SymbolInformation;
use serde::{Deserialize, Serialize};
use tracing::{debug, warn};

/// Serializable representation of a cached symbol index.
#[derive(Serialize, Deserialize)]
struct CacheFile {
    /// Per-file: URI → last modified (seconds since epoch).
    file_mtimes: HashMap<String, u64>,
    /// All symbols from the index.
    symbols: Vec<SymbolInformation>,
}

/// Directory inside the workspace for cache files.
const CACHE_DIR: &str = ".cache/programmer-mcp";

/// Get the cache file path for a given language.
fn cache_path(workspace: &Path, language: &str) -> PathBuf {
    workspace
        .join(CACHE_DIR)
        .join(format!("{language}.symbols.json"))
}

/// Save the current symbol index to disk.
pub fn save(workspace: &Path, language: &str, symbols: &[SymbolInformation]) {
    let path = cache_path(workspace, language);
    if let Some(parent) = path.parent() {
        if let Err(e) = std::fs::create_dir_all(parent) {
            warn!("failed to create cache dir: {e}");
            return;
        }
    }

    // Collect file mtimes from the symbols
    let mut file_mtimes = HashMap::new();
    for sym in symbols {
        let uri = sym.location.uri.as_str();
        if file_mtimes.contains_key(uri) {
            continue;
        }
        if let Some(abs) = uri.strip_prefix("file://") {
            if let Ok(meta) = std::fs::metadata(abs) {
                if let Ok(mtime) = meta.modified() {
                    if let Ok(secs) = mtime.duration_since(SystemTime::UNIX_EPOCH) {
                        file_mtimes.insert(uri.to_string(), secs.as_secs());
                    }
                }
            }
        }
    }

    let cache = CacheFile {
        file_mtimes,
        symbols: symbols.to_vec(),
    };

    match serde_json::to_string(&cache) {
        Ok(json) => {
            if let Err(e) = std::fs::write(&path, json) {
                warn!("failed to write symbol cache: {e}");
            } else {
                debug!(
                    language,
                    symbols = cache.symbols.len(),
                    "saved symbol cache"
                );
            }
        }
        Err(e) => warn!("failed to serialize symbol cache: {e}"),
    }
}

/// Load cached symbols from disk, returning only those from unchanged files.
/// Also returns the list of file URIs that need re-indexing.
pub fn load(workspace: &Path, language: &str) -> Option<(Vec<SymbolInformation>, Vec<String>)> {
    let path = cache_path(workspace, language);
    let data = std::fs::read_to_string(&path).ok()?;
    let cache: CacheFile = serde_json::from_str(&data).ok()?;

    let mut valid_symbols = Vec::new();
    let mut stale_uris = Vec::new();

    // Check each file's mtime
    let mut checked_files: HashMap<&str, bool> = HashMap::new();
    for (uri, cached_mtime) in &cache.file_mtimes {
        let is_fresh = if let Some(abs) = uri.strip_prefix("file://") {
            std::fs::metadata(abs)
                .and_then(|m| m.modified())
                .and_then(|t| {
                    t.duration_since(SystemTime::UNIX_EPOCH)
                        .map_err(|e| std::io::Error::other(e.to_string()))
                })
                .map(|d| d.as_secs() == *cached_mtime)
                .unwrap_or(false)
        } else {
            false
        };
        checked_files.insert(uri.as_str(), is_fresh);
        if !is_fresh {
            stale_uris.push(uri.clone());
        }
    }

    // Keep symbols from fresh files only
    for sym in &cache.symbols {
        let uri = sym.location.uri.as_str();
        if checked_files.get(uri).copied().unwrap_or(false) {
            valid_symbols.push(sym.clone());
        }
    }

    debug!(
        language,
        total = cache.symbols.len(),
        valid = valid_symbols.len(),
        stale = stale_uris.len(),
        "loaded symbol cache"
    );

    Some((valid_symbols, stale_uris))
}
