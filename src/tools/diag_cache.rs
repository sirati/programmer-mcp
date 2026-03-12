//! Background diagnostics cache.
//!
//! The watcher triggers diagnostics for changed files. Results are cached
//! alongside file hashes in `.programmer-mcp/.cache/`. New diagnostics
//! discovered since the last check are reported on the next `execute` call.

use std::collections::HashMap;
use std::fmt::Write;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use lsp_types::{Diagnostic, DiagnosticSeverity};
use tokio::sync::RwLock;

/// Stores cached diagnostics per file, keyed by file path.
#[derive(Default)]
struct CacheEntry {
    hash: u64,
    diagnostics: Vec<Diagnostic>,
}

pub struct DiagCache {
    cache_dir: PathBuf,
    entries: RwLock<HashMap<String, CacheEntry>>,
    /// New diagnostics accumulated since last `take_new`.
    pending: RwLock<Vec<String>>,
}

impl DiagCache {
    pub fn new(workspace: &Path) -> Arc<Self> {
        let cache_dir = workspace.join(".programmer-mcp/.cache/diagnostics");
        let _ = std::fs::create_dir_all(&cache_dir);
        Arc::new(Self {
            cache_dir,
            entries: RwLock::new(HashMap::new()),
            pending: RwLock::new(Vec::new()),
        })
    }

    /// Update diagnostics for a file. If diagnostics changed, queue a notification.
    pub async fn update(&self, file_path: &str, diagnostics: Vec<Diagnostic>) {
        let content_hash = match tokio::fs::read(file_path).await {
            Ok(bytes) => hash_bytes(&bytes),
            Err(_) => return,
        };

        let mut entries = self.entries.write().await;
        let entry = entries.entry(file_path.to_string()).or_default();

        // Skip if file hasn't changed and diagnostics are the same count
        if entry.hash == content_hash && entry.diagnostics.len() == diagnostics.len() {
            return;
        }

        // Find new diagnostics (present now but not before)
        let new_diags: Vec<&Diagnostic> = diagnostics
            .iter()
            .filter(|d| !entry.diagnostics.iter().any(|old| diag_eq(old, d)))
            .collect();

        if !new_diags.is_empty() {
            let summary = format_new_diags(file_path, &new_diags);
            self.pending.write().await.push(summary);
        }

        entry.hash = content_hash;
        entry.diagnostics = diagnostics;

        // Persist to cache dir
        let _ = self.persist_entry(file_path, entry);
    }

    /// Take all pending new-diagnostic notifications, clearing the queue.
    pub async fn take_pending(&self) -> Vec<String> {
        std::mem::take(&mut *self.pending.write().await)
    }

    fn persist_entry(&self, file_path: &str, entry: &CacheEntry) -> std::io::Result<()> {
        let safe_name = file_path.replace('/', "__");
        let cache_file = self.cache_dir.join(format!("{safe_name}.json"));
        let data = serde_json::json!({
            "hash": entry.hash,
            "count": entry.diagnostics.len(),
        });
        std::fs::write(cache_file, data.to_string())
    }
}

fn hash_bytes(bytes: &[u8]) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    bytes.hash(&mut hasher);
    hasher.finish()
}

fn diag_eq(a: &Diagnostic, b: &Diagnostic) -> bool {
    a.range == b.range && a.message == b.message && a.severity == b.severity
}

fn format_new_diags(file_path: &str, diags: &[&Diagnostic]) -> String {
    let mut out = format!("New diagnostics in {file_path}:\n");
    for d in diags {
        let severity = match d.severity {
            Some(DiagnosticSeverity::ERROR) => "Error",
            Some(DiagnosticSeverity::WARNING) => "Warning",
            Some(DiagnosticSeverity::INFORMATION) => "Info",
            Some(DiagnosticSeverity::HINT) => "Hint",
            _ => "Unknown",
        };
        let _ = writeln!(
            out,
            "  {severity} at L{}:C{}: {}",
            d.range.start.line + 1,
            d.range.start.character + 1,
            d.message,
        );
    }
    out
}
