use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;

use tracing::{error, info};

use super::client::{LspClient, LspClientError};
use super::detect_lang::detect_language_id;
use crate::config::LspSpec;

/// Manages multiple LSP clients, routing requests by language.
pub struct LspManager {
    clients: HashMap<String, Arc<LspClient>>,
}

impl LspManager {
    /// Spawn and initialize all configured LSP servers.
    pub async fn start(
        specs: &[LspSpec],
        workspace: &Path,
    ) -> Result<Self, LspClientError> {
        let mut clients = HashMap::new();

        for spec in specs {
            info!(language = %spec.language, command = %spec.command, "starting LSP");
            match LspClient::start(&spec.language, &spec.command, &spec.args, workspace).await {
                Ok(client) => {
                    if let Err(e) = client.initialize(workspace).await {
                        error!(language = %spec.language, "LSP initialization failed: {e}");
                        continue;
                    }
                    clients.insert(spec.language.clone(), Arc::new(client));
                }
                Err(e) => {
                    error!(language = %spec.language, "failed to start LSP: {e}");
                }
            }
        }

        if clients.is_empty() {
            return Err(LspClientError::Other(
                "no LSP servers started successfully".into(),
            ));
        }

        Ok(Self { clients })
    }

    /// Get a specific client by language name.
    pub fn get(&self, language: &str) -> Option<&Arc<LspClient>> {
        self.clients.get(language)
    }

    /// Get the client for a file path based on its extension.
    pub fn for_file(&self, path: &str) -> Option<&Arc<LspClient>> {
        let lang = detect_language_id(path);
        if lang.is_empty() {
            return None;
        }
        // Try exact match first, then check aliases
        self.clients
            .get(lang)
            .or_else(|| self.clients.values().find(|c| c.language() == lang))
    }

    /// Get all clients (for broadcast operations).
    pub fn all(&self) -> impl Iterator<Item = &Arc<LspClient>> {
        self.clients.values()
    }

    /// Resolve which client(s) to use for an operation.
    ///
    /// If `language` is specified, return just that client.
    /// If `file_path` is specified, detect language from extension.
    /// Otherwise return all clients.
    pub fn resolve(
        &self,
        language: Option<&str>,
        file_path: Option<&str>,
    ) -> Vec<&Arc<LspClient>> {
        if let Some(lang) = language {
            self.get(lang).into_iter().collect()
        } else if let Some(path) = file_path {
            self.for_file(path).into_iter().collect()
        } else {
            self.all().collect()
        }
    }

    /// Shutdown all LSP clients.
    pub async fn shutdown(&self) {
        for client in self.clients.values() {
            let _ = client.shutdown().await;
        }
    }
}
