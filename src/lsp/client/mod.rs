//! LSP client — lifecycle, initialisation, and diagnostics cache.
//!
//! File-synchronisation methods live in [`file_sync`]; LSP query methods
//! (workspace/document symbol, hover, references, rename, …) live in
//! [`requests`].

mod file_sync;
mod requests;

use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;

use jsonrpsee::async_client::{Client, ClientBuilder};
use jsonrpsee::core::client::{ClientT, SubscriptionClientT};
use jsonrpsee::core::traits::ToRpcParams;
use lsp_types::notification::{Exit, Initialized, Notification, PublishDiagnostics};
use lsp_types::request::{Initialize, Request, Shutdown};
use lsp_types::{
    ClientInfo, Diagnostic, InitializeParams, InitializeResult, InitializedParams, OneOf,
    PublishDiagnosticsParams, Uri,
};
use serde::Serialize;
use serde_json::value::RawValue;
use tokio::process::Command;
use tokio::sync::RwLock;
use tracing::{debug, info, trace, warn};

use super::capabilities::build_client_capabilities;
use super::transport;
use crate::tools::symbol_cache::SymbolCache;

// ── Helpers ──────────────────────────────────────────────────────────

struct RpcParams<T: Serialize>(T);

impl<T: Serialize> ToRpcParams for RpcParams<T> {
    fn to_rpc_params(self) -> Result<Option<Box<RawValue>>, serde_json::Error> {
        let json = serde_json::to_string(&self.0)?;
        RawValue::from_string(json).map(Some)
    }
}

#[derive(thiserror::Error, Debug)]
pub enum LspClientError {
    #[error("jsonrpc error: {0}")]
    Rpc(#[from] jsonrpsee::core::client::Error),
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("{0}")]
    Other(String),
}

// ── Open file tracking ──────────────────────────────────────────────

struct OpenFileInfo {
    version: i32,
}

// ── LspClient ───────────────────────────────────────────────────────

pub struct LspClient {
    rpc: Client,
    language: String,
    open_files: Arc<RwLock<HashMap<String, OpenFileInfo>>>,
    diagnostics: Arc<RwLock<HashMap<String, Vec<Diagnostic>>>>,
    symbol_cache: SymbolCache,
    has_workspace_symbol: std::sync::atomic::AtomicBool,
    _child: tokio::process::Child,
}

impl LspClient {
    pub async fn start(
        language: &str,
        command: &str,
        args: &[String],
        workspace: &Path,
    ) -> Result<Self, LspClientError> {
        let mut child = Command::new(command)
            .args(args)
            .current_dir(workspace)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .kill_on_drop(true)
            .spawn()?;

        let stdin = child.stdin.take().expect("stdin piped");
        let stdout = child.stdout.take().expect("stdout piped");
        let stderr = child.stderr.take().expect("stderr piped");

        let lang_for_log = language.to_string();
        tokio::spawn(async move {
            use tokio::io::{AsyncBufReadExt, BufReader};
            let mut lines = BufReader::new(stderr).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                trace!(lsp = %lang_for_log, stderr = %line);
            }
        });

        let (sender, receiver) = transport::io_transport(stdin, stdout).await;
        let rpc = ClientBuilder::default()
            .request_timeout(std::time::Duration::from_secs(120))
            .build_with_tokio(sender, receiver);

        Ok(Self {
            rpc,
            language: language.to_string(),
            open_files: Arc::new(RwLock::new(HashMap::new())),
            diagnostics: Arc::new(RwLock::new(HashMap::new())),
            symbol_cache: SymbolCache::new(),
            has_workspace_symbol: std::sync::atomic::AtomicBool::new(false),
            _child: child,
        })
    }

    pub async fn initialize(&self, workspace: &Path) -> Result<InitializeResult, LspClientError> {
        let workspace_str = workspace.to_string_lossy();
        let workspace_uri: Uri = format!("file://{workspace_str}")
            .parse()
            .map_err(|e| LspClientError::Other(format!("bad workspace URI: {e}")))?;

        #[allow(deprecated)] // root_uri needed for older LSP servers
        let params = InitializeParams {
            process_id: Some(std::process::id()),
            root_uri: Some(workspace_uri.clone()),
            // Don't send workspaceFolders — some LSPs (basedpyright) will
            // attempt to index the entire workspace, which blocks responses
            // when the workspace is a large non-matching project (e.g. Rust).
            // rootUri is sufficient for file resolution.
            workspace_folders: None,
            capabilities: build_client_capabilities(),
            initialization_options: None,
            client_info: Some(ClientInfo {
                name: "programmer-mcp".into(),
                version: Some(env!("CARGO_PKG_VERSION").into()),
            }),
            locale: None,
            ..Default::default()
        };

        // Subscribe before initialize so we don't miss early notifications.
        self.subscribe_diagnostics().await;
        self.subscribe_progress().await;
        self.subscribe_noise_notifications().await;

        let result: InitializeResult = self
            .rpc
            .request(Initialize::METHOD, RpcParams(params))
            .await?;

        self.rpc
            .notification(Initialized::METHOD, RpcParams(InitializedParams {}))
            .await?;

        // Record whether server supports workspace/symbol.
        let has_ws_sym = result
            .capabilities
            .workspace_symbol_provider
            .as_ref()
            .map(|p| match p {
                OneOf::Left(b) => *b,
                OneOf::Right(_) => true,
            })
            .unwrap_or(false);
        self.has_workspace_symbol
            .store(has_ws_sym, std::sync::atomic::Ordering::Relaxed);

        info!(language = %self.language, has_workspace_symbol = has_ws_sym, "LSP initialized");

        Ok(result)
    }

    async fn subscribe_diagnostics(&self) {
        let diag_store = self.diagnostics.clone();
        let lang = self.language.clone();

        match self
            .rpc
            .subscribe_to_method::<PublishDiagnosticsParams>(PublishDiagnostics::METHOD)
            .await
        {
            Ok(mut sub) => {
                tokio::spawn(async move {
                    while let Some(Ok(params)) = sub.next().await {
                        let count = params.diagnostics.len();
                        let uri_key = params.uri.as_str().to_string();
                        diag_store
                            .write()
                            .await
                            .insert(uri_key.clone(), params.diagnostics);
                        trace!(lsp = %lang, uri = %uri_key, count, "diagnostics updated");
                    }
                });
            }
            Err(e) => {
                warn!(lsp = %lang, "failed to subscribe to diagnostics: {e}");
            }
        }
    }

    /// Subscribe to $/progress to log LSP indexing status and detect when ready.
    async fn subscribe_progress(&self) {
        let lang = self.language.clone();
        if let Ok(mut sub) = self
            .rpc
            .subscribe_to_method::<serde_json::Value>("$/progress")
            .await
        {
            tokio::spawn(async move {
                while let Some(Ok(val)) = sub.next().await {
                    // Extract progress message for logging
                    if let Some(value) = val.get("value") {
                        let kind = value.get("kind").and_then(|k| k.as_str()).unwrap_or("");
                        let message = value.get("message").and_then(|m| m.as_str());
                        let title = value.get("title").and_then(|t| t.as_str());
                        match kind {
                            "begin" => {
                                let title = title.unwrap_or("work");
                                info!(lsp = %lang, title, "LSP started: {title}");
                            }
                            "end" => {
                                let msg = message.unwrap_or("done");
                                info!(lsp = %lang, "LSP finished: {msg}");
                            }
                            "report" => {
                                if let Some(msg) = message {
                                    debug!(lsp = %lang, "{msg}");
                                }
                            }
                            _ => {}
                        }
                    }
                }
            });
        }
    }

    /// Subscribe to noisy notifications to prevent "not a registered method" log spam.
    /// Note: server→client *requests* (workspace/configuration, client/registerCapability,
    /// window/workDoneProgress/create) are handled automatically in the transport layer.
    async fn subscribe_noise_notifications(&self) {
        let methods = ["workspace/diagnostic/refresh", "window/logMessage"];
        for method in methods {
            if let Ok(mut sub) = self
                .rpc
                .subscribe_to_method::<serde_json::Value>(method)
                .await
            {
                tokio::spawn(async move { while let Some(Ok(_)) = sub.next().await {} });
            }
        }
    }

    pub async fn get_cached_diagnostics(&self, uri: &Uri) -> Vec<Diagnostic> {
        let key = uri.as_str().to_string();
        self.diagnostics
            .read()
            .await
            .get(&key)
            .cloned()
            .unwrap_or_default()
    }

    pub async fn shutdown(&self) -> Result<(), LspClientError> {
        if let Err(e) = self
            .rpc
            .request::<(), _>(Shutdown::METHOD, RpcParams(()))
            .await
        {
            warn!(lsp = %self.language, "shutdown request failed: {e}");
        }
        if let Err(e) = self.rpc.notification(Exit::METHOD, RpcParams(())).await {
            warn!(lsp = %self.language, "exit notification failed: {e}");
        }
        info!(lsp = %self.language, "LSP shut down");
        Ok(())
    }

    pub fn language(&self) -> &str {
        &self.language
    }

    pub fn symbol_cache(&self) -> &SymbolCache {
        &self.symbol_cache
    }

    pub fn has_workspace_symbol(&self) -> bool {
        self.has_workspace_symbol
            .load(std::sync::atomic::Ordering::Relaxed)
    }
}
