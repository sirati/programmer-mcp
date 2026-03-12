use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;

use tracing::{error, info, warn};

use super::client::{LspClient, LspClientError};
use super::detect_lang::detect_language_id;
use crate::config::LspSpec;
use crate::nix::NixEnv;

/// Manages multiple LSP clients, routing requests by language.
pub struct LspManager {
    clients: HashMap<String, Arc<LspClient>>,
}

impl LspManager {
    /// Spawn and initialize all configured LSP servers.
    ///
    /// If a command is not found in PATH and nix+flakes are available,
    /// a `nix run nixpkgs#{pkg}` fallback is attempted automatically.
    pub async fn start(specs: &[LspSpec], workspace: &Path) -> Result<Self, LspClientError> {
        let nix = NixEnv::detect().await;
        let mut clients = HashMap::new();

        for spec in specs {
            info!(language = %spec.language, command = %spec.command, "starting LSP");
            if let Some(client) = start_one_spec(spec, workspace, &nix).await {
                clients.insert(spec.language.clone(), Arc::new(client));
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
    pub fn resolve(&self, language: Option<&str>, file_path: Option<&str>) -> Vec<&Arc<LspClient>> {
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

/// Try to start and initialize a single LSP spec, with automatic nix fallback.
///
/// Returns `Some(client)` on success, `None` if startup failed.
async fn start_one_spec(spec: &LspSpec, workspace: &Path, nix: &NixEnv) -> Option<LspClient> {
    // First attempt: use the spec as given.
    match try_start_and_init(&spec.language, &spec.command, &spec.args, workspace).await {
        Ok(client) => return Some(client),
        Err(e) if !is_not_found_error(&e) => {
            error!(language = %spec.language, "failed to start LSP: {e}");
            return None;
        }
        Err(_) => {
            warn!(
                language = %spec.language,
                command  = %spec.command,
                "LSP command not found, trying nix fallback"
            );
        }
    }

    // Second attempt: nix run fallback.
    let Some((nix_cmd, nix_args)) = nix.fallback_command(&spec.command, &spec.args) else {
        error!(
            language = %spec.language,
            command  = %spec.command,
            "LSP command not found and nix+flakes are not available"
        );
        return None;
    };

    info!(language = %spec.language, nix_cmd = %nix_cmd, ?nix_args, "starting LSP via nix run");
    match try_start_and_init(&spec.language, &nix_cmd, &nix_args, workspace).await {
        Ok(client) => Some(client),
        Err(e) => {
            error!(language = %spec.language, "failed to start LSP via nix: {e}");
            None
        }
    }
}

/// Start the LSP process and run the initialize handshake.
async fn try_start_and_init(
    language: &str,
    command: &str,
    args: &[String],
    workspace: &Path,
) -> Result<LspClient, LspClientError> {
    let client = LspClient::start(language, command, args, workspace).await?;
    client.initialize(workspace).await?;
    Ok(client)
}

/// Heuristic to determine if an `LspClientError` represents a "command not
/// found" OS-level failure (as opposed to a protocol error after the process
/// started).
fn is_not_found_error(e: &LspClientError) -> bool {
    match e {
        LspClientError::Io(io_err) => io_err.kind() == std::io::ErrorKind::NotFound,
        LspClientError::Other(msg) => {
            let lower = msg.to_lowercase();
            lower.contains("not found")
                || lower.contains("no such file")
                || lower.contains("os error 2")
        }
        _ => false,
    }
}
