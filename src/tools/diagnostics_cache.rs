//! Persistent diagnostics cache with change-based reporting.
//!
//! When files change, the watcher triggers diagnostics. Results are compared
//! against a cached hash + diagnostics snapshot. New diagnostics are queued
//! as "pending" and drained on the next `execute` call.

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use tokio::sync::Mutex;

/// Cached diagnostics state for a single file.
#[derive(serde::Serialize, serde::Deserialize)]
struct CacheEntry {
    content_hash: u64,
    diagnostics: Vec<String>,
}

/// Shared diagnostics cache with pending-report queue.
pub struct DiagnosticsCache {
    cache_dir: PathBuf,
    pending: Mutex<Vec<String>>,
}

impl DiagnosticsCache {
    pub fn new(workspace: &Path) -> Arc<Self> {
        let cache_dir = workspace.join(".programmer-mcp").join(".cache");
        std::fs::create_dir_all(&cache_dir).ok();
        Arc::new(Self {
            cache_dir,
            pending: Mutex::new(Vec::new()),
        })
    }

    /// Update cache for a file. If diagnostics changed, queue a pending report.
    pub async fn update(&self, file_path: &str, diagnostics: Vec<String>) {
        let content_hash = match std::fs::read_to_string(file_path) {
            Ok(content) => hash_string(&content),
            Err(_) => return,
        };

        let cache_path = self.cache_path(file_path);
        let old = self.load_entry(&cache_path);

        // Skip if content unchanged and diagnostics are the same
        if let Some(ref old) = old {
            if old.content_hash == content_hash && old.diagnostics == diagnostics {
                return;
            }
        }

        let new_entry = CacheEntry {
            content_hash,
            diagnostics: diagnostics.clone(),
        };
        self.save_entry(&cache_path, &new_entry);

        // Find new diagnostics not in the old set
        let old_set: std::collections::HashSet<&str> = old
            .as_ref()
            .map(|e| e.diagnostics.iter().map(|s| s.as_str()).collect())
            .unwrap_or_default();

        let new_diags: Vec<&str> = diagnostics
            .iter()
            .filter(|d| !old_set.contains(d.as_str()))
            .map(|s| s.as_str())
            .collect();

        if !new_diags.is_empty() {
            let report = format!(
                "--- Auto-diagnostics: {} ---\n{}",
                file_path,
                new_diags.join("\n")
            );
            self.pending.lock().await.push(report);
        }
    }

    /// Drain all pending diagnostic reports.
    pub async fn take_pending(&self) -> Vec<String> {
        std::mem::take(&mut *self.pending.lock().await)
    }

    fn cache_path(&self, file_path: &str) -> PathBuf {
        // Use a hash of the file path to avoid path separator issues
        let mut h = DefaultHasher::new();
        file_path.hash(&mut h);
        self.cache_dir.join(format!("{:x}.json", h.finish()))
    }

    fn load_entry(&self, path: &Path) -> Option<CacheEntry> {
        let data = std::fs::read_to_string(path).ok()?;
        serde_json::from_str(&data).ok()
    }

    fn save_entry(&self, path: &Path, entry: &CacheEntry) {
        if let Ok(data) = serde_json::to_string(entry) {
            std::fs::write(path, data).ok();
        }
    }
}

fn hash_string(s: &str) -> u64 {
    let mut h = DefaultHasher::new();
    s.hash(&mut h);
    h.finish()
}

/// Extract diagnostic summary strings from LSP diagnostics for caching.
pub fn format_diagnostics_for_cache(
    file_path: &str,
    diagnostics: &[lsp_types::Diagnostic],
) -> Vec<String> {
    diagnostics
        .iter()
        .map(|d| {
            let severity = match d.severity {
                Some(lsp_types::DiagnosticSeverity::ERROR) => "error",
                Some(lsp_types::DiagnosticSeverity::WARNING) => "warning",
                Some(lsp_types::DiagnosticSeverity::INFORMATION) => "info",
                Some(lsp_types::DiagnosticSeverity::HINT) => "hint",
                _ => "diagnostic",
            };
            format!(
                "{}:{}:{}: {}: {}",
                file_path,
                d.range.start.line + 1,
                d.range.start.character + 1,
                severity,
                d.message
            )
        })
        .collect()
}
