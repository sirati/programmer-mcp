//! LSP query request methods.

use std::time::Duration;

use jsonrpsee::core::client::ClientT;
use jsonrpsee::core::traits::ToRpcParams;
use lsp_types::{
    request::*, CodeActionContext, CodeActionOrCommand, CodeActionParams, DocumentDiagnosticParams,
    DocumentFormattingParams, DocumentSymbolParams, DocumentSymbolResponse, FormattingOptions,
    Hover, HoverParams, Location, Position, Range, ReferenceContext, ReferenceParams, RenameParams,
    SymbolInformation, TextDocumentIdentifier, TextDocumentPositionParams, TextEdit, Uri,
    WorkspaceEdit, WorkspaceSymbolParams,
};
use serde::de::DeserializeOwned;

use super::{LspClient, LspClientError, RpcParams};

/// Timeout for individual LSP requests.
const LSP_REQUEST_TIMEOUT: Duration = Duration::from_secs(3);

impl LspClient {
    /// Send an LSP request with a timeout. Returns an error if the LSP doesn't
    /// respond within [`LSP_REQUEST_TIMEOUT`].
    async fn timed_request<R: DeserializeOwned>(
        &self,
        method: &str,
        params: impl ToRpcParams + Send,
    ) -> Result<R, LspClientError> {
        match tokio::time::timeout(LSP_REQUEST_TIMEOUT, self.rpc.request(method, params)).await {
            Ok(result) => Ok(result?),
            Err(_elapsed) => Err(LspClientError::Other(format!(
                "LSP request '{method}' timed out after {}s (language={})",
                LSP_REQUEST_TIMEOUT.as_secs(),
                self.language()
            ))),
        }
    }

    pub async fn workspace_symbol(
        &self,
        query: &str,
    ) -> Result<Vec<SymbolInformation>, LspClientError> {
        let params = WorkspaceSymbolParams {
            query: query.to_string(),
            ..Default::default()
        };

        let raw: serde_json::Value = self
            .timed_request(WorkspaceSymbolRequest::METHOD, RpcParams(params))
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

        tracing::debug!(lsp = %self.language(), uri = ?uri, "sending documentSymbol request");
        let result: DocumentSymbolResponse = self
            .timed_request(DocumentSymbolRequest::METHOD, RpcParams(params))
            .await?;
        tracing::debug!(lsp = %self.language(), "documentSymbol response received");

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

        self.timed_request(References::METHOD, RpcParams(params))
            .await
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

        self.timed_request(HoverRequest::METHOD, RpcParams(params))
            .await
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

        self.timed_request(Rename::METHOD, RpcParams(params)).await
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
            .timed_request("textDocument/diagnostic", RpcParams(params))
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

    /// Get available code actions at a range, optionally filtered by kind.
    pub async fn code_action(
        &self,
        uri: &Uri,
        range: Range,
        only: Option<Vec<lsp_types::CodeActionKind>>,
    ) -> Result<Option<Vec<CodeActionOrCommand>>, LspClientError> {
        let diagnostics = self.get_cached_diagnostics(uri).await;
        let relevant_diags: Vec<_> = diagnostics
            .into_iter()
            .filter(|d| ranges_overlap(&d.range, &range))
            .collect();

        let params = CodeActionParams {
            text_document: TextDocumentIdentifier { uri: uri.clone() },
            range,
            context: CodeActionContext {
                diagnostics: relevant_diags,
                only,
                ..Default::default()
            },
            work_done_progress_params: Default::default(),
            partial_result_params: Default::default(),
        };

        self.timed_request(CodeActionRequest::METHOD, RpcParams(params))
            .await
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
            .timed_request(Formatting::METHOD, RpcParams(params))
            .await?;
        Ok(result.unwrap_or_default())
    }

    /// Prepare call hierarchy at a position.
    pub async fn call_hierarchy_prepare(
        &self,
        uri: &Uri,
        position: Position,
    ) -> Result<Option<Vec<lsp_types::CallHierarchyItem>>, LspClientError> {
        let params = lsp_types::CallHierarchyPrepareParams {
            text_document_position_params: TextDocumentPositionParams {
                text_document: TextDocumentIdentifier { uri: uri.clone() },
                position,
            },
            work_done_progress_params: Default::default(),
        };
        self.timed_request(CallHierarchyPrepare::METHOD, RpcParams(params))
            .await
    }

    /// Get incoming calls (callers) for a call hierarchy item.
    pub async fn call_hierarchy_incoming(
        &self,
        item: lsp_types::CallHierarchyItem,
    ) -> Result<Option<Vec<lsp_types::CallHierarchyIncomingCall>>, LspClientError> {
        let params = lsp_types::CallHierarchyIncomingCallsParams {
            item,
            work_done_progress_params: Default::default(),
            partial_result_params: Default::default(),
        };
        self.timed_request(CallHierarchyIncomingCalls::METHOD, RpcParams(params))
            .await
    }

    /// Get outgoing calls (callees) from a call hierarchy item.
    pub async fn call_hierarchy_outgoing(
        &self,
        item: lsp_types::CallHierarchyItem,
    ) -> Result<Option<Vec<lsp_types::CallHierarchyOutgoingCall>>, LspClientError> {
        let params = lsp_types::CallHierarchyOutgoingCallsParams {
            item,
            work_done_progress_params: Default::default(),
            partial_result_params: Default::default(),
        };
        self.timed_request(CallHierarchyOutgoingCalls::METHOD, RpcParams(params))
            .await
    }

    /// Send an arbitrary LSP request and return the raw JSON response.
    pub async fn raw_request(
        &self,
        method: &str,
        params: serde_json::Value,
    ) -> Result<serde_json::Value, LspClientError> {
        self.timed_request(method, RpcParams(params)).await
    }
}

fn ranges_overlap(a: &Range, b: &Range) -> bool {
    a.start <= b.end && b.start <= a.end
}
