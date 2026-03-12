//! Code actions and formatting — expose LSP refactoring capabilities.

use std::sync::Arc;

use crate::lsp::client::{LspClient, LspClientError};

use super::formatting::path_to_uri;
use super::rename::{apply_text_edits, apply_workspace_edit};

/// Get available code actions at a position in a file.
pub async fn get_code_actions(
    client: &Arc<LspClient>,
    file_path: &str,
    line: u32,
    column: u32,
) -> Result<String, LspClientError> {
    client.open_file(file_path).await?;
    let actions = client.code_actions(file_path, line, column).await?;
    if actions.is_empty() {
        return Ok("No code actions available at this position.".into());
    }
    let mut out = String::new();
    for (i, action) in actions.iter().enumerate() {
        let kind = action.kind.as_deref().unwrap_or("action");
        out.push_str(&format!("[{i}] ({kind}) {}\n", action.title));
    }
    Ok(out.trim_end().to_string())
}

/// Apply a code action by index at a position.
pub async fn apply_code_action(
    client: &Arc<LspClient>,
    file_path: &str,
    line: u32,
    column: u32,
    index: usize,
) -> Result<String, LspClientError> {
    client.open_file(file_path).await?;
    let actions = client.code_actions(file_path, line, column).await?;
    let action = actions.get(index).ok_or_else(|| {
        LspClientError::Other(format!(
            "action index {index} out of range (available: {})",
            actions.len()
        ))
    })?;
    if let Some(edit) = &action.edit {
        apply_workspace_edit(edit)?;
    }
    Ok(format!("Applied: {}", action.title))
}

/// Format an entire file.
pub async fn format_file(
    client: &Arc<LspClient>,
    file_path: &str,
) -> Result<String, LspClientError> {
    client.open_file(file_path).await?;
    let edits = client.format_raw(file_path).await?;
    if edits.is_empty() {
        return Ok("File already formatted (no changes).".into());
    }
    let count = edits.len();
    let uri = path_to_uri(file_path).map_err(LspClientError::Other)?;
    apply_text_edits(&uri, &edits)?;
    Ok(format!("Applied {count} formatting edits."))
}
