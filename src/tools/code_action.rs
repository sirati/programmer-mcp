//! Code action (refactoring / quick-fix) support.

use std::fmt::Write;
use std::sync::Arc;

use lsp_types::{CodeActionKind, CodeActionOrCommand, Range};

use super::formatting::{path_to_uri, to_lsp_position};
use crate::lsp::client::{LspClient, LspClientError};

/// Get available code actions at a position or range.
pub async fn get_code_actions(
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
