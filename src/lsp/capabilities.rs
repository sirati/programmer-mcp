//! LSP client capability declaration.

use lsp_types::{
    ClientCapabilities, DidChangeWatchedFilesClientCapabilities, DocumentSymbolClientCapabilities,
    DynamicRegistrationClientCapabilities, GotoCapability, HoverClientCapabilities,
    PublishDiagnosticsClientCapabilities, RenameClientCapabilities, TextDocumentClientCapabilities,
    TextDocumentSyncClientCapabilities, WorkspaceClientCapabilities,
    WorkspaceSymbolClientCapabilities,
};

/// Build the `ClientCapabilities` advertised to LSP servers on initialization.
pub fn build_client_capabilities() -> ClientCapabilities {
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
