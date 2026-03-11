use std::path::Path;
use std::sync::Arc;

use notify::{Event, EventKind, RecursiveMode, Watcher};
use tokio::sync::mpsc;
use tracing::{debug, error, info};

use crate::lsp::manager::LspManager;
use crate::tools::formatting::path_to_uri;

/// Watch workspace for file changes and notify LSP servers.
pub async fn watch_workspace(manager: Arc<LspManager>, workspace: &Path) {
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

                for client in manager.all() {
                    if let Err(e) = client.did_change_watched_files(changes.clone()).await {
                        debug!(lsp = %client.language(), "watched files notification failed: {e}");
                    }

                    for change in &changes {
                        if change.typ == lsp_types::FileChangeType::CHANGED {
                            let uri_str = change.uri.as_str();
                            if let Some(path) = uri_str.strip_prefix("file://") {
                                if let Err(e) = client.notify_file_changed(path).await {
                                    debug!(lsp = %client.language(), path, "file change notify failed: {e}");
                                }
                            }
                        }
                    }
                }
            }
            _ => {}
        }
    }

    // Keep watcher alive
    drop(watcher);
}
