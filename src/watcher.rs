use std::path::Path;
use std::sync::Arc;

use notify::{Event, EventKind, RecursiveMode, Watcher};
use tokio::sync::mpsc;
use tracing::{debug, error, info};

use crate::lsp::manager::LspManager;
use crate::tools::diagnostics_cache::{diagnostics_to_entries, DiagnosticsCache};
use crate::tools::formatting::path_to_uri;

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

    info!(path = %workspace.display(), "watching workspace for changes");

    while let Some(event) = rx.recv().await {
        match event.kind {
            EventKind::Create(_) | EventKind::Modify(_) | EventKind::Remove(_) => {
                let changes: Vec<lsp_types::FileEvent> = event
                    .paths
                    .iter()
                    .filter_map(|p| {
                        let path_str = p.to_string_lossy();
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

                debug!(count = changes.len(), "file change events");

                notify_and_collect(&manager, &diag_cache, &changes).await;
            }
            _ => {}
        }
    }

    drop(watcher);
}

async fn notify_and_collect(
    manager: &Arc<LspManager>,
    diag_cache: &Arc<DiagnosticsCache>,
    changes: &[lsp_types::FileEvent],
) {
    // Notify LSP servers of file changes and invalidate symbol caches
    for client in manager.all() {
        if let Err(e) = client.did_change_watched_files(changes.to_vec()).await {
            debug!(lsp = %client.language(), "watched files notification failed: {e}");
        }
        for change in changes {
            let uri_str = change.uri.as_str();
            // Invalidate symbol cache for source file changes (skip build artifacts)
            if !uri_str.contains("/target/") {
                client.symbol_cache().invalidate_file(uri_str).await;
            }
            if change.typ == lsp_types::FileChangeType::CHANGED {
                if let Some(path) = uri_str.strip_prefix("file://") {
                    if let Err(e) = client.notify_file_changed(path).await {
                        debug!(lsp = %client.language(), path, "file change notify failed: {e}");
                    }
                }
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
        // Wait for LSP to process the changes
        tokio::time::sleep(std::time::Duration::from_secs(3)).await;
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
