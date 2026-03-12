//! File synchronisation methods – notifying the LSP when files are opened or modified.

use jsonrpsee::core::client::ClientT;
use lsp_types::{
    notification::*, DidChangeTextDocumentParams, DidChangeWatchedFilesParams,
    DidOpenTextDocumentParams, FileEvent, TextDocumentContentChangeEvent, TextDocumentItem,
    VersionedTextDocumentIdentifier,
};

use super::super::detect_lang::detect_language_id;
use super::{LspClient, LspClientError, OpenFileInfo, RpcParams};
use crate::tools::formatting::path_to_uri;

impl LspClient {
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
}
