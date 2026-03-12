//! LSP query request methods.

use jsonrpsee::core::client::ClientT;
use lsp_types::{
    request::*, CodeActionContext, CodeActionOrCommand, CodeActionParams, DocumentDiagnosticParams,
    DocumentFormattingParams, DocumentSymbolParams, DocumentSymbolResponse, FormattingOptions,
    GotoDefinitionParams, GotoDefinitionResponse, Hover, HoverParams, Location, Position, Range,
    ReferenceContext, ReferenceParams, RenameParams, SymbolInformation, TextDocumentIdentifier,
    TextDocumentPositionParams, TextEdit, Uri, WorkspaceEdit, WorkspaceSymbolParams,
};

use super::{LspClient, LspClientError, RpcParams};

/// Simplified code action info returned to the tools layer.
pub struct CodeActionInfo {
    pub title: String,
    pub kind: Option<String>,
    pub edit: Option<WorkspaceEdit>,
    pub command: Option<lsp_types::Command>,
}

impl LspClient {
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

        Ok(serde_json::from_value(raw).unwrap_or_default())
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

    pub async fn goto_definition(
        &self,
        uri: &Uri,
        position: Position,
    ) -> Result<Option<GotoDefinitionResponse>, LspClientError> {
        let params = GotoDefinitionParams {
            text_document_position_params: TextDocumentPositionParams {
                text_document: TextDocumentIdentifier { uri: uri.clone() },
                position,
            },
            work_done_progress_params: Default::default(),
            partial_result_params: Default::default(),
        };

        let result: Option<GotoDefinitionResponse> = self
            .rpc
            .request(GotoDefinition::METHOD, RpcParams(params))
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
                tracing::debug!(lsp = %self.language, "pull diagnostics not supported: {e}");
                return Ok(());
            }
        };

        Ok(())
    }

    /// Get available code actions at a position.
    pub async fn code_actions(
        &self,
        file_path: &str,
        line: u32,
        column: u32,
    ) -> Result<Vec<CodeActionInfo>, LspClientError> {
        let uri =
            crate::tools::formatting::path_to_uri(file_path).map_err(LspClientError::Other)?;
        let pos = Position::new(line.saturating_sub(1), column.saturating_sub(1));
        let range = Range::new(pos, pos);
        let diagnostics = self.get_cached_diagnostics(&uri).await;
        let params = CodeActionParams {
            text_document: TextDocumentIdentifier { uri },
            range,
            context: CodeActionContext {
                diagnostics,
                only: None,
                trigger_kind: None,
            },
            work_done_progress_params: Default::default(),
            partial_result_params: Default::default(),
        };
        let result: Option<Vec<CodeActionOrCommand>> = self
            .rpc
            .request(CodeActionRequest::METHOD, RpcParams(params))
            .await?;
        Ok(result
            .unwrap_or_default()
            .into_iter()
            .filter_map(|a| match a {
                CodeActionOrCommand::CodeAction(ca) => Some(CodeActionInfo {
                    title: ca.title,
                    kind: ca.kind.map(|k| k.as_str().to_string()),
                    edit: ca.edit,
                    command: ca.command,
                }),
                CodeActionOrCommand::Command(cmd) => Some(CodeActionInfo {
                    title: cmd.title.clone(),
                    kind: None,
                    edit: None,
                    command: Some(cmd),
                }),
            })
            .collect())
    }

    /// Format a document, returning the text edits (caller applies them).
    pub async fn format_raw(&self, file_path: &str) -> Result<Vec<TextEdit>, LspClientError> {
        let uri =
            crate::tools::formatting::path_to_uri(file_path).map_err(LspClientError::Other)?;
        let params = DocumentFormattingParams {
            text_document: TextDocumentIdentifier { uri },
            options: FormattingOptions {
                tab_size: 4,
                insert_spaces: true,
                ..Default::default()
            },
            work_done_progress_params: Default::default(),
        };
        let result: Option<Vec<TextEdit>> = self
            .rpc
            .request(Formatting::METHOD, RpcParams(params))
            .await?;
        Ok(result.unwrap_or_default())
    }

    /// Send an arbitrary LSP request and return the raw JSON response.
    pub async fn raw_request(
        &self,
        method: &str,
        params: serde_json::Value,
    ) -> Result<serde_json::Value, LspClientError> {
        let result: serde_json::Value = self.rpc.request(method, RpcParams(params)).await?;
        Ok(result)
    }
}
