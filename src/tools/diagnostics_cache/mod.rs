//! Persistent diagnostics cache with change-based reporting.
//!
//! When files change, the watcher triggers diagnostics. Results are compared
//! against a cached hash + diagnostics snapshot. New diagnostics are queued
//! as "pending" and drained on the next `execute` call.
//!
//! Language-specific processing (noise stripping, message normalization) lives
//! in submodules like [`rust`].

mod format;
pub mod rust;

use std::collections::HashSet;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use tokio::sync::Mutex;

/// A single structured diagnostic entry for caching and diffing.
#[derive(Clone, Debug, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub struct DiagEntry {
    pub severity: String,
    pub line: u32,
    pub col: u32,
    pub message: String,
}

/// Cached diagnostics state for a single file.
#[derive(serde::Serialize, serde::Deserialize)]
struct CacheEntry {
    content_hash: u64,
    diagnostics: Vec<DiagEntry>,
}

/// A pending new diagnostic tied to its file path.
pub(crate) struct PendingDiag {
    pub file_path: String,
    pub entry: DiagEntry,
}

/// Shared diagnostics cache with pending-report queue.
pub struct DiagnosticsCache {
    cache_dir: PathBuf,
    workspace_root: PathBuf,
    pending: Mutex<Vec<PendingDiag>>,
}

impl DiagnosticsCache {
    pub fn new(workspace: &Path) -> Arc<Self> {
        let cache_dir = workspace.join(".programmer-mcp").join(".cache");
        std::fs::create_dir_all(&cache_dir).ok();
        Arc::new(Self {
            cache_dir,
            workspace_root: workspace.to_path_buf(),
            pending: Mutex::new(Vec::new()),
        })
    }

    /// Update cache for a file. If diagnostics changed, queue new entries as pending.
    pub async fn update(&self, file_path: &str, diagnostics: Vec<DiagEntry>) {
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
        let old_set: HashSet<&DiagEntry> = old
            .as_ref()
            .map(|e| e.diagnostics.iter().collect())
            .unwrap_or_default();

        let new_diags: Vec<&DiagEntry> = diagnostics
            .iter()
            .filter(|d| !old_set.contains(d))
            .collect();

        if !new_diags.is_empty() {
            let mut pending = self.pending.lock().await;
            for entry in new_diags {
                pending.push(PendingDiag {
                    file_path: file_path.to_string(),
                    entry: entry.clone(),
                });
            }
        }
    }

    /// Drain all pending diagnostics and format them compactly.
    /// Returns `None` if there are no pending diagnostics.
    pub async fn take_pending(&self) -> Option<String> {
        let items = std::mem::take(&mut *self.pending.lock().await);
        if items.is_empty() {
            return None;
        }

        // Deduplicate: same file + same entry
        let mut seen = HashSet::new();
        let mut deduped = Vec::new();
        for item in items {
            let key = (item.file_path.clone(), item.entry.clone());
            if seen.insert(key) {
                deduped.push(item);
            }
        }

        Some(format::format_pending(&self.workspace_root, deduped))
    }

    fn cache_path(&self, file_path: &str) -> PathBuf {
        let mut h = std::collections::hash_map::DefaultHasher::new();
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
    let mut h = std::collections::hash_map::DefaultHasher::new();
    s.hash(&mut h);
    h.finish()
}

/// Extract structured diagnostic entries from LSP diagnostics.
/// Uses language-specific processing when `language` is provided.
pub fn diagnostics_to_entries(
    diagnostics: &[lsp_types::Diagnostic],
    language: Option<&str>,
) -> Vec<DiagEntry> {
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
            let message = match language {
                Some("rust") => rust::clean_message(&d.message),
                _ => strip_noise_generic(&d.message),
            };
            DiagEntry {
                severity: severity.to_string(),
                line: d.range.start.line + 1,
                col: d.range.start.character + 1,
                message,
            }
        })
        .collect()
}

/// Generic noise stripping — removes blank trailing lines.
fn strip_noise_generic(message: &str) -> String {
    message.trim().to_string()
}
