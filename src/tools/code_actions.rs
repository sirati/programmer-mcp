//! Code actions and formatting — expose LSP refactoring capabilities.

use std::fmt::Write;
use std::sync::Arc;

use lsp_types::{CodeActionKind, CodeActionOrCommand, Range};

use crate::lsp::client::{LspClient, LspClientError};

use super::formatting::{path_to_uri, to_lsp_position};
use super::rename::{apply_text_edits, apply_workspace_edit};

/// Get available code actions at a position in a file (simple).
pub async fn get_code_actions(
    client: &Arc<LspClient>,
    file_path: &str,
    line: u32,
    column: u32,
) -> Result<String, LspClientError> {
    get_code_actions_range(client, file_path, line, column, None, None, &[]).await
}

/// Get available code actions at a position or range, with optional kind filtering.
pub async fn get_code_actions_range(
    client: &Arc<LspClient>,
    file_path: &str,
    line: u32,
    column: u32,
    end_line: Option<u32>,
    end_column: Option<u32>,
    kinds: &[String],
) -> Result<String, LspClientError> {
    client.open_file(file_path).await?;

    let uri = path_to_uri(file_path).map_err(LspClientError::Other)?;
    let start = to_lsp_position(line, column);
    let end = to_lsp_position(end_line.unwrap_or(line), end_column.unwrap_or(column));
    let range = Range { start, end };

    let only = if kinds.is_empty() {
        None
    } else {
        Some(
            kinds
                .iter()
                .map(|k| CodeActionKind::from(k.clone()))
                .collect(),
        )
    };

    let actions = client.code_action(&uri, range, only).await?;

    let actions = match actions {
        Some(a) if !a.is_empty() => a,
        _ => {
            return Ok(format!(
                "No code actions available at {file_path}:L{line}:C{column}"
            ))
        }
    };

    let mut output = format!(
        "{file_path}:L{line}:C{column}\nAvailable code actions: {}\n\n",
        actions.len()
    );

    for (i, action) in actions.iter().enumerate() {
        match action {
            CodeActionOrCommand::CodeAction(ca) => {
                let kind = ca.kind.as_ref().map(|k| k.as_str()).unwrap_or("unknown");
                let _ = writeln!(output, "[{i}] ({kind}) {}", ca.title);
                if let Some(edit) = &ca.edit {
                    let file_count = edit.changes.as_ref().map(|c| c.len()).unwrap_or(0)
                        + edit
                            .document_changes
                            .as_ref()
                            .map(|dc| match dc {
                                lsp_types::DocumentChanges::Edits(e) => e.len(),
                                lsp_types::DocumentChanges::Operations(o) => o.len(),
                            })
                            .unwrap_or(0);
                    if file_count > 0 {
                        let _ = writeln!(output, "     affects {file_count} file(s)");
                    }
                }
            }
            CodeActionOrCommand::Command(cmd) => {
                let _ = writeln!(output, "[{i}] (command) {}", cmd.title);
            }
        }
    }

    Ok(output)
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

    let uri = path_to_uri(file_path).map_err(LspClientError::Other)?;
    let pos = to_lsp_position(line, column);
    let range = Range {
        start: pos,
        end: pos,
    };
    let actions = client.code_action(&uri, range, None).await?;
    let actions = actions.unwrap_or_default();
    let action = actions.get(index).ok_or_else(|| {
        LspClientError::Other(format!(
            "action index {index} out of range (available: {})",
            actions.len()
        ))
    })?;
    match action {
        CodeActionOrCommand::CodeAction(ca) => {
            if let Some(edit) = &ca.edit {
                apply_workspace_edit(edit)?;
            }
            Ok(format!("Applied: {}", ca.title))
        }
        CodeActionOrCommand::Command(cmd) => Ok(format!(
            "Command '{}' requires server-side execution",
            cmd.title
        )),
    }
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
