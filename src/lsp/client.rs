use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;

use jsonrpsee::async_client::{Client, ClientBuilder};
use jsonrpsee::core::client::{ClientT, SubscriptionClientT};
use jsonrpsee::core::traits::ToRpcParams;
use lsp_types::notification::*;
use lsp_types::request::*;
use lsp_types::*;
use serde::Serialize;
use serde_json::value::RawValue;
use tokio::process::Command;
use tokio::sync::RwLock;
use tracing::{debug, info, warn};

use super::detect_lang::detect_language_id;
use super::transport;
use crate::tools::formatting::{path_to_uri, uri_to_path};

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

        // Log stderr in background
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

        let client = Self {
            rpc,
            language: language.to_string(),
            open_files: Arc::new(RwLock::new(HashMap::new())),
            diagnostics: Arc::new(RwLock::new(HashMap::new())),
            _child: child,
        };

        Ok(client)
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
            capabilities: self.client_capabilities(),
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

        // Send initialized notification
        self.rpc
            .notification(Initialized::METHOD, RpcParams(InitializedParams {}))
            .await?;

        info!(language = %self.language, "LSP initialized");

        // Start diagnostics subscription
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

    fn client_capabilities(&self) -> ClientCapabilities {
        ClientCapabilities {
            workspace: Some(WorkspaceClientCapabilities {
                configuration: Some(true),
                did_change_configuration: Some(DynamicRegistrationClientCapabilities {
                    dynamic_registration: Some(true),
                }),
                did_change_watched_files: Some(DidChangeWatchedFilesClientCapabilities {
                    dynamic_registration: Some(true),
                    relative_pattern_support: Some(true),
                }),
                workspace_folders: Some(true),
                symbol: Some(WorkspaceSymbolClientCapabilities {
                    dynamic_registration: Some(true),
                    ..Default::default()
                }),
                apply_edit: Some(true),
                ..Default::default()
            }),
            text_document: Some(TextDocumentClientCapabilities {
                synchronization: Some(TextDocumentSyncClientCapabilities {
                    dynamic_registration: Some(true),
                    did_save: Some(true),
                    ..Default::default()
                }),
                publish_diagnostics: Some(PublishDiagnosticsClientCapabilities {
                    version_support: Some(true),
                    ..Default::default()
                }),
                document_symbol: Some(DocumentSymbolClientCapabilities {
                    dynamic_registration: Some(true),
                    ..Default::default()
                }),
                rename: Some(RenameClientCapabilities {
                    dynamic_registration: Some(true),
                    prepare_support: Some(true),
                    ..Default::default()
                }),
                hover: Some(HoverClientCapabilities {
                    dynamic_registration: Some(true),
                    ..Default::default()
                }),
                references: Some(DynamicRegistrationClientCapabilities {
                    dynamic_registration: Some(true),
                }),
                definition: Some(GotoCapability {
                    dynamic_registration: Some(true),
                    ..Default::default()
                }),
                ..Default::default()
            }),
            ..Default::default()
        }
    }

    // ── File management ─────────────────────────────────────────────

    pub async fn open_file(&self, path: &str) -> Result<(), LspClientError> {
        let uri = path_to_uri(path).map_err(LspClientError::Other)?;
        let uri_key = uri.as_str().to_string();

        {
            let files = self.open_files.read().await;
            if files.contains_key(&uri_key) {
                return Ok(());
            }
        }

        let content = tokio::fs::read_to_string(path).await?;
        let language_id = detect_language_id(path).to_string();

        let params = DidOpenTextDocumentParams {
            text_document: TextDocumentItem {
                uri,
                language_id,
                version: 1,
                text: content,
            },
        };

        self.rpc
            .notification(DidOpenTextDocument::METHOD, RpcParams(params))
            .await?;

        self.open_files
            .write()
            .await
            .insert(uri_key, OpenFileInfo { version: 1 });

        Ok(())
    }

    pub async fn notify_file_changed(&self, path: &str) -> Result<(), LspClientError> {
        let uri = path_to_uri(path).map_err(LspClientError::Other)?;
        let uri_key = uri.as_str().to_string();

        let mut files = self.open_files.write().await;
        if let Some(info) = files.get_mut(&uri_key) {
            let content = tokio::fs::read_to_string(path).await?;
            info.version += 1;

            let params = DidChangeTextDocumentParams {
                text_document: VersionedTextDocumentIdentifier {
                    uri,
                    version: info.version,
                },
                content_changes: vec![TextDocumentContentChangeEvent {
                    range: None,
                    range_length: None,
                    text: content,
                }],
            };

            self.rpc
                .notification(DidChangeTextDocument::METHOD, RpcParams(params))
                .await?;
        }

        Ok(())
    }

    // ── LSP requests ────────────────────────────────────────────────

    pub async fn workspace_symbol(
        &self,
        query: &str,
    ) -> Result<Vec<SymbolInformation>, LspClientError> {
        let params = WorkspaceSymbolParams {
            query: query.to_string(),
            ..Default::default()
        };

        let raw: serde_json::Value = self
            .rpc
            .request(WorkspaceSymbolRequest::METHOD, RpcParams(params))
            .await?;

        let symbols: Vec<SymbolInformation> = serde_json::from_value(raw).unwrap_or_default();

        Ok(symbols)
    }

    pub async fn document_symbol(
        &self,
        uri: &Uri,
    ) -> Result<DocumentSymbolResponse, LspClientError> {
        let params = DocumentSymbolParams {
            text_document: TextDocumentIdentifier { uri: uri.clone() },
            work_done_progress_params: Default::default(),
            partial_result_params: Default::default(),
        };

        let result: DocumentSymbolResponse = self
            .rpc
            .request(DocumentSymbolRequest::METHOD, RpcParams(params))
            .await?;

        Ok(result)
    }

    pub async fn references(
        &self,
        uri: &Uri,
        position: Position,
        include_declaration: bool,
    ) -> Result<Option<Vec<Location>>, LspClientError> {
        let params = ReferenceParams {
            text_document_position: TextDocumentPositionParams {
                text_document: TextDocumentIdentifier { uri: uri.clone() },
                position,
            },
            context: ReferenceContext {
                include_declaration,
            },
            work_done_progress_params: Default::default(),
            partial_result_params: Default::default(),
        };

        let result: Option<Vec<Location>> = self
            .rpc
            .request(References::METHOD, RpcParams(params))
            .await?;

        Ok(result)
    }

    pub async fn hover(
        &self,
        uri: &Uri,
        position: Position,
    ) -> Result<Option<Hover>, LspClientError> {
        let params = HoverParams {
            text_document_position_params: TextDocumentPositionParams {
                text_document: TextDocumentIdentifier { uri: uri.clone() },
                position,
            },
            work_done_progress_params: Default::default(),
        };

        let result: Option<Hover> = self
            .rpc
            .request(HoverRequest::METHOD, RpcParams(params))
            .await?;

        Ok(result)
    }

    pub async fn rename(
        &self,
        uri: &Uri,
        position: Position,
        new_name: &str,
    ) -> Result<Option<WorkspaceEdit>, LspClientError> {
        let params = RenameParams {
            text_document_position: TextDocumentPositionParams {
                text_document: TextDocumentIdentifier { uri: uri.clone() },
                position,
            },
            new_name: new_name.to_string(),
            work_done_progress_params: Default::default(),
        };

        let result: Option<WorkspaceEdit> =
            self.rpc.request(Rename::METHOD, RpcParams(params)).await?;

        Ok(result)
    }

    pub async fn diagnostic(&self, uri: &Uri) -> Result<(), LspClientError> {
        let params = DocumentDiagnosticParams {
            text_document: TextDocumentIdentifier { uri: uri.clone() },
            identifier: None,
            previous_result_id: None,
            work_done_progress_params: Default::default(),
            partial_result_params: Default::default(),
        };

        let _: serde_json::Value = match self
            .rpc
            .request("textDocument/diagnostic", RpcParams(params))
            .await
        {
            Ok(v) => v,
            Err(e) => {
                debug!(lsp = %self.language, "pull diagnostics not supported: {e}");
                return Ok(());
            }
        };

        Ok(())
    }

    // ── Diagnostics cache access ────────────────────────────────────

    pub async fn get_cached_diagnostics(&self, uri: &Uri) -> Vec<Diagnostic> {
        let key = uri.as_str().to_string();
        self.diagnostics
            .read()
            .await
            .get(&key)
            .cloned()
            .unwrap_or_default()
    }

    // ── Watched file events ─────────────────────────────────────────

    pub async fn did_change_watched_files(
        &self,
        changes: Vec<FileEvent>,
    ) -> Result<(), LspClientError> {
        let params = DidChangeWatchedFilesParams { changes };
        self.rpc
            .notification(DidChangeWatchedFiles::METHOD, RpcParams(params))
            .await?;
        Ok(())
    }

    // ── Shutdown ────────────────────────────────────────────────────

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
