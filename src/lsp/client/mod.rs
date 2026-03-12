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
use lsp_types::PublishDiagnosticsParams;
use lsp_types::{
    ClientInfo, Diagnostic, InitializeParams, InitializeResult, InitializedParams, Uri,
    WorkspaceFolder,
};
use serde::Serialize;
use serde_json::value::RawValue;
use tokio::process::Command;
use tokio::sync::RwLock;
use tracing::{debug, info, warn};

use super::capabilities::build_client_capabilities;
use super::transport;

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
                debug!(lsp = %lang_for_log, stderr = %line);
            }
        });

        let (sender, receiver) = transport::io_transport(stdin, stdout);
        let rpc = ClientBuilder::default().build_with_tokio(sender, receiver);

        Ok(Self {
            rpc,
            language: language.to_string(),
            open_files: Arc::new(RwLock::new(HashMap::new())),
            diagnostics: Arc::new(RwLock::new(HashMap::new())),
            _child: child,
        })
    }

    pub async fn initialize(&self, workspace: &Path) -> Result<InitializeResult, LspClientError> {
        let workspace_str = workspace.to_string_lossy();
        let workspace_uri: Uri = format!("file://{workspace_str}")
            .parse()
            .map_err(|e| LspClientError::Other(format!("bad workspace URI: {e}")))?;

        let params = InitializeParams {
            process_id: Some(std::process::id()),
            root_uri: Some(workspace_uri.clone()),
            root_path: Some(workspace_str.into()),
            workspace_folders: Some(vec![WorkspaceFolder {
                uri: workspace_uri,
                name: workspace
                    .file_name()
                    .map(|n| n.to_string_lossy().into())
                    .unwrap_or_else(|| "workspace".into()),
            }]),
            capabilities: build_client_capabilities(),
            initialization_options: None,
            client_info: Some(ClientInfo {
                name: "programmer-mcp".into(),
                version: Some(env!("CARGO_PKG_VERSION").into()),
            }),
            locale: None,
            ..Default::default()
        };

        let result: InitializeResult = self
            .rpc
            .request(Initialize::METHOD, RpcParams(params))
            .await?;

        self.rpc
            .notification(Initialized::METHOD, RpcParams(InitializedParams {}))
            .await?;

        info!(language = %self.language, "LSP initialized");
        self.subscribe_diagnostics().await;

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
                    use futures::StreamExt;
                    while let Some(Ok(params)) = sub.next().await {
                        let count = params.diagnostics.len();
                        let uri_key = params.uri.as_str().to_string();
                        diag_store
                            .write()
                            .await
                            .insert(uri_key.clone(), params.diagnostics);
                        debug!(lsp = %lang, uri = %uri_key, count, "diagnostics updated");
                    }
                });
            }
            Err(e) => {
                warn!(lsp = %lang, "failed to subscribe to diagnostics: {e}");
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
}
