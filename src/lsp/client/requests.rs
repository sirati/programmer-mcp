//! LSP query request methods.

use jsonrpsee::core::client::ClientT;
use lsp_types::{
    request::*, DocumentDiagnosticParams, DocumentSymbolParams, DocumentSymbolResponse,
    GotoDefinitionParams, GotoDefinitionResponse, Hover, HoverParams, Location, Position,
    ReferenceContext, ReferenceParams, RenameParams, SymbolInformation, TextDocumentIdentifier,
    TextDocumentPositionParams, Uri, WorkspaceEdit, WorkspaceSymbolParams,
};

use super::{LspClient, LspClientError, RpcParams};

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

    pub async fn code_action(
        &self,
        uri: &Uri,
        range: lsp_types::Range,
        only: Option<Vec<lsp_types::CodeActionKind>>,
    ) -> Result<Option<Vec<lsp_types::CodeActionOrCommand>>, LspClientError> {
        let diagnostics = self.get_cached_diagnostics(uri).await;
        // Filter diagnostics that overlap the range
        let relevant_diags: Vec<_> = diagnostics
            .into_iter()
            .filter(|d| ranges_overlap(&d.range, &range))
            .collect();

        let params = lsp_types::CodeActionParams {
            text_document: TextDocumentIdentifier { uri: uri.clone() },
            range,
            context: lsp_types::CodeActionContext {
                diagnostics: relevant_diags,
                only,
                ..Default::default()
            },
            work_done_progress_params: Default::default(),
            partial_result_params: Default::default(),
        };

        let result: Option<Vec<lsp_types::CodeActionOrCommand>> = self
            .rpc
            .request(
                lsp_types::request::CodeActionRequest::METHOD,
                RpcParams(params),
            )
            .await?;

        Ok(result)
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

fn ranges_overlap(a: &lsp_types::Range, b: &lsp_types::Range) -> bool {
    a.start <= b.end && b.start <= a.end
}
