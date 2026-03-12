use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

use ignore::gitignore::Gitignore;
use notify::{Event, EventKind, RecursiveMode, Watcher};
use tokio::sync::mpsc;
use tracing::{debug, error, info, trace};

use crate::lsp::detect_lang::detect_language_id;
use crate::lsp::manager::LspManager;
use crate::tools::diagnostics_cache::{diagnostics_to_entries, DiagnosticsCache};
use crate::tools::formatting::path_to_uri;

// ── Per-file backoff ─────────────────────────────────────────────────

const BACKOFF_MIN: Duration = Duration::from_secs(1);
const BACKOFF_MAX: Duration = Duration::from_secs(15);
const BACKOFF_RESET_MULTIPLIER: u32 = 4;

struct FileBackoff {
    entries: HashMap<String, BackoffEntry>,
}

struct BackoffEntry {
    current: Duration,
    last_event: Instant,
}

impl FileBackoff {
    fn new() -> Self {
        Self {
            entries: HashMap::new(),
        }
    }

    /// Returns true if this file event should be processed (not throttled).
    fn should_process(&mut self, path: &str) -> bool {
        let now = Instant::now();
        let entry = self
            .entries
            .entry(path.to_string())
            .or_insert(BackoffEntry {
                current: BACKOFF_MIN,
                last_event: now - BACKOFF_MAX * 2, // ensure first event always passes
            });

        let elapsed = now.duration_since(entry.last_event);

        // Reset backoff if idle for 4x the current backoff
        if elapsed >= entry.current * BACKOFF_RESET_MULTIPLIER {
            entry.current = BACKOFF_MIN;
            entry.last_event = now;
            return true;
        }

        // Throttle if within backoff window
        if elapsed < entry.current {
            return false;
        }

        // Process and increase backoff
        entry.last_event = now;
        entry.current = (entry.current * 2).min(BACKOFF_MAX);
        true
    }
}

// ── Language relevance ───────────────────────────────────────────────

/// Known config files that are relevant to specific LSP languages.
fn config_file_language(filename: &str) -> Option<&'static str> {
    match filename {
        "pyrightconfig.json" | "pyproject.toml" | "setup.py" | "setup.cfg" | ".python-version" => {
            Some("python")
        }
        "Cargo.toml"
        | "Cargo.lock"
        | "rust-toolchain.toml"
        | "rust-toolchain"
        | "clippy.toml"
        | "rustfmt.toml"
        | ".rustfmt.toml" => Some("rust"),
        "go.mod" | "go.sum" => Some("go"),
        "package.json" | "tsconfig.json" | "jsconfig.json" | ".eslintrc.json" | ".prettierrc" => {
            Some("typescript")
        }
        "flake.nix" | "flake.lock" | "default.nix" | "shell.nix" => Some("nix"),
        _ => None,
    }
}

/// Determine which LSP language a file change is relevant to.
/// Returns None if the file is not relevant to any LSP.
fn file_relevant_language(path: &str) -> Option<&'static str> {
    // Check if it's a known config file
    let filename = Path::new(path)
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("");
    if let Some(lang) = config_file_language(filename) {
        return Some(lang);
    }

    // Check by extension
    let lang = detect_language_id(path);
    if lang.is_empty() {
        return None;
    }
    Some(lang)
}

// ── Watcher ──────────────────────────────────────────────────────────

/// Watch workspace for file changes, notify LSP servers, and auto-collect diagnostics.
pub async fn watch_workspace(
    manager: Arc<LspManager>,
    diag_cache: Arc<DiagnosticsCache>,
    workspace: &Path,
) {
    let (tx, mut rx) = mpsc::channel::<Event>(256);

    let mut watcher = match notify::recommended_watcher(move |res: Result<Event, _>| {
        if let Ok(event) = res {
            let _ = tx.blocking_send(event);
        }
    }) {
        Ok(w) => w,
        Err(e) => {
            error!("failed to create file watcher: {e}");
            return;
        }
    };

    if let Err(e) = watcher.watch(workspace, RecursiveMode::Recursive) {
        error!("failed to watch workspace: {e}");
        return;
    }

    // Build gitignore matcher from workspace
    let gitignore = build_gitignore(workspace);
    let mut backoff = FileBackoff::new();

    info!(path = %workspace.display(), "watching workspace for changes");

    while let Some(event) = rx.recv().await {
        match event.kind {
            EventKind::Create(_) | EventKind::Modify(_) | EventKind::Remove(_) => {
                let changes: Vec<lsp_types::FileEvent> = event
                    .paths
                    .iter()
                    .filter_map(|p| {
                        let path_str = p.to_string_lossy();

                        // Skip gitignored files
                        let is_dir = p.is_dir();
                        if gitignore.matched_path_or_any_parents(p, is_dir).is_ignore() {
                            return None;
                        }

                        // Skip common non-source dirs not covered by gitignore
                        if path_str.contains("/.git/") {
                            return None;
                        }

                        // Per-file backoff
                        if !backoff.should_process(&path_str) {
                            return None;
                        }

                        let uri = path_to_uri(&path_str).ok()?;
                        let change_type = match event.kind {
                            EventKind::Create(_) => lsp_types::FileChangeType::CREATED,
                            EventKind::Remove(_) => lsp_types::FileChangeType::DELETED,
                            _ => lsp_types::FileChangeType::CHANGED,
                        };
                        Some(lsp_types::FileEvent {
                            uri,
                            typ: change_type,
                        })
                    })
                    .collect();

                if changes.is_empty() {
                    continue;
                }

                trace!(count = changes.len(), "file change events");

                notify_and_collect(&manager, &diag_cache, &changes).await;
            }
            _ => {}
        }
    }

    drop(watcher);
}

fn build_gitignore(workspace: &Path) -> Gitignore {
    let mut builder = ignore::gitignore::GitignoreBuilder::new(workspace);

    // Always ignore common dirs even without .gitignore
    let _ = builder.add_line(None, "/target/");
    let _ = builder.add_line(None, "/node_modules/");
    let _ = builder.add_line(None, "/__pycache__/");
    let _ = builder.add_line(None, "/.programmer-mcp/");

    // Load .gitignore if present
    let gitignore_path = workspace.join(".gitignore");
    if gitignore_path.exists() {
        let _ = builder.add(&gitignore_path);
    }

    builder.build().unwrap_or_else(|e| {
        debug!("failed to build gitignore: {e}, using defaults");
        ignore::gitignore::GitignoreBuilder::new(workspace)
            .build()
            .unwrap()
    })
}

async fn notify_and_collect(
    manager: &Arc<LspManager>,
    diag_cache: &Arc<DiagnosticsCache>,
    changes: &[lsp_types::FileEvent],
) {
    // Group changes by relevant language, then notify only matching LSP clients
    for client in manager.all() {
        let lang = client.language();
        let relevant: Vec<lsp_types::FileEvent> = changes
            .iter()
            .filter(|change| {
                let uri_str = change.uri.as_str();
                let path = uri_str.strip_prefix("file://").unwrap_or(uri_str);
                match file_relevant_language(path) {
                    Some(file_lang) => file_lang == lang,
                    None => false, // unknown file type → don't notify any LSP
                }
            })
            .cloned()
            .collect();

        if relevant.is_empty() {
            continue;
        }

        if let Err(e) = client.did_change_watched_files(relevant.clone()).await {
            debug!(lsp = %lang, "watched files notification failed: {e}");
        }

        for change in &relevant {
            let uri_str = change.uri.as_str();
            client.symbol_cache().invalidate_file(uri_str).await;

            if change.typ == lsp_types::FileChangeType::CHANGED {
                if let Some(path) = uri_str.strip_prefix("file://") {
                    if let Err(e) = client.notify_file_changed(path).await {
                        debug!(lsp = %lang, path, "file change notify failed: {e}");
                    }
                }
                // Re-index changed file in background after LSP processes the change.
                let c = client.clone();
                let uri = change.uri.clone();
                tokio::spawn(async move {
                    tokio::time::sleep(Duration::from_secs(2)).await;
                    if let Err(e) = c.symbol_cache().index_file(&c, &uri).await {
                        trace!(lsp = %c.language(), "re-index after change failed: {e}");
                    }
                });
            }
        }
    }

    // Schedule delayed diagnostics collection for changed files
    let changed_paths: Vec<String> = changes
        .iter()
        .filter(|c| c.typ != lsp_types::FileChangeType::DELETED)
        .filter_map(|c| c.uri.as_str().strip_prefix("file://").map(String::from))
        .collect();

    if changed_paths.is_empty() {
        return;
    }

    let mgr = manager.clone();
    let cache = diag_cache.clone();
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_secs(3)).await;
        collect_diagnostics(&mgr, &cache, &changed_paths).await;
    });
}

async fn collect_diagnostics(manager: &LspManager, cache: &DiagnosticsCache, paths: &[String]) {
    for path in paths {
        let clients = manager.resolve(None, Some(path));
        for client in clients {
            let uri = match path_to_uri(path) {
                Ok(u) => u,
                Err(_) => continue,
            };
            let diagnostics = client.get_cached_diagnostics(&uri).await;
            let entries = diagnostics_to_entries(&diagnostics, Some(client.language()));
            cache.update(path, entries).await;
        }
    }
}
