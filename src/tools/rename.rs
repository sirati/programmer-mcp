use std::collections::BTreeMap;
use std::fmt::Write;
use std::sync::Arc;

use lsp_types::{DocumentChangeOperation, DocumentChanges, OneOf, TextEdit, Uri};

use super::formatting::{path_to_uri, to_lsp_position, uri_to_path};
use crate::lsp::client::{LspClient, LspClientError};

/// Rename a symbol at a given position and apply changes to the workspace.
pub async fn rename_symbol(
    client: &Arc<LspClient>,
    file_path: &str,
    line: u32,
    column: u32,
    new_name: &str,
) -> Result<String, LspClientError> {
    client.open_file(file_path).await?;

    let uri = path_to_uri(file_path).map_err(LspClientError::Other)?;
    let position = to_lsp_position(line, column);

    let edit = client.rename(&uri, position, new_name).await?;

    let edit = match edit {
        Some(e) => e,
        None => return Ok("Failed to rename symbol. No edit returned.".into()),
    };

    let mut change_count = 0u32;
    let mut file_details: BTreeMap<String, Vec<String>> = BTreeMap::new();

    // Process Changes field
    if let Some(changes) = &edit.changes {
        for (change_uri, edits) in changes {
            let path = uri_to_path(change_uri).unwrap_or_else(|| change_uri.as_str().to_string());
            let locs: Vec<String> = edits
                .iter()
                .map(|e| {
                    format!(
                        "L{}:C{}",
                        e.range.start.line + 1,
                        e.range.start.character + 1
                    )
                })
                .collect();
            change_count += edits.len() as u32;
            file_details.entry(path).or_default().extend(locs);
        }
    }

    // Process DocumentChanges field
    if let Some(doc_changes) = &edit.document_changes {
        match doc_changes {
            DocumentChanges::Edits(text_doc_edits) => {
                for tde in text_doc_edits {
                    let path = uri_to_path(&tde.text_document.uri)
                        .unwrap_or_else(|| tde.text_document.uri.as_str().to_string());
                    let locs: Vec<String> = tde
                        .edits
                        .iter()
                        .filter_map(|e| match e {
                            OneOf::Left(te) => Some(format!(
                                "L{}:C{}",
                                te.range.start.line + 1,
                                te.range.start.character + 1
                            )),
                            OneOf::Right(ate) => Some(format!(
                                "L{}:C{}",
                                ate.text_edit.range.start.line + 1,
                                ate.text_edit.range.start.character + 1
                            )),
                        })
                        .collect();
                    change_count += tde.edits.len() as u32;
                    file_details.entry(path).or_default().extend(locs);
                }
            }
            DocumentChanges::Operations(ops) => {
                for op in ops {
                    if let DocumentChangeOperation::Edit(tde) = op {
                        let path = uri_to_path(&tde.text_document.uri)
                            .unwrap_or_else(|| tde.text_document.uri.as_str().to_string());
                        let locs: Vec<String> = tde
                            .edits
                            .iter()
                            .filter_map(|e| match e {
                                OneOf::Left(te) => Some(format!(
                                    "L{}:C{}",
                                    te.range.start.line + 1,
                                    te.range.start.character + 1
                                )),
                                OneOf::Right(ate) => Some(format!(
                                    "L{}:C{}",
                                    ate.text_edit.range.start.line + 1,
                                    ate.text_edit.range.start.character + 1
                                )),
                            })
                            .collect();
                        change_count += tde.edits.len() as u32;
                        file_details.entry(path).or_default().extend(locs);
                    }
                }
            }
        }
    }

    let file_count = file_details.len() as u32;

    if file_count == 0 || change_count == 0 {
        return Ok("Failed to rename symbol. 0 occurrences found.".into());
    }

    // Apply changes to disk
    apply_workspace_edit(&edit)?;

    let mut details = String::new();
    for (path, locs) in &file_details {
        let _ = writeln!(details, "{path}: {}", locs.join(", "));
    }

    Ok(format!(
        "Successfully renamed symbol to '{new_name}'.\n\
         Updated {change_count} occurrences across {file_count} files:\n{details}"
    ))
}

pub fn apply_workspace_edit(edit: &lsp_types::WorkspaceEdit) -> Result<(), LspClientError> {
    if let Some(changes) = &edit.changes {
        for (uri, edits) in changes {
            apply_text_edits(uri, edits)?;
        }
    }

    if let Some(doc_changes) = &edit.document_changes {
        let text_doc_edits: Vec<_> = match doc_changes {
            DocumentChanges::Edits(edits) => edits.clone(),
            DocumentChanges::Operations(ops) => ops
                .iter()
                .filter_map(|op| match op {
                    DocumentChangeOperation::Edit(e) => Some(e.clone()),
                    _ => None,
                })
                .collect(),
        };

        for tde in &text_doc_edits {
            let edits: Vec<TextEdit> = tde
                .edits
                .iter()
                .map(|e| match e {
                    OneOf::Left(te) => te.clone(),
                    OneOf::Right(ate) => ate.text_edit.clone(),
                })
                .collect();
            apply_text_edits(&tde.text_document.uri, &edits)?;
        }
    }

    Ok(())
}

pub fn apply_text_edits(uri: &Uri, edits: &[TextEdit]) -> Result<(), LspClientError> {
    let path = uri_to_path(uri).ok_or_else(|| LspClientError::Other("invalid URI".into()))?;

    let content = std::fs::read_to_string(&path)
        .map_err(|e| LspClientError::Other(format!("read error: {e}")))?;

    let mut lines: Vec<String> = content.lines().map(String::from).collect();

    // Sort edits in reverse order so line numbers stay valid
    let mut sorted_edits: Vec<&TextEdit> = edits.iter().collect();
    sorted_edits.sort_by(|a, b| {
        b.range
            .start
            .line
            .cmp(&a.range.start.line)
            .then(b.range.start.character.cmp(&a.range.start.character))
    });

    for edit in sorted_edits {
        let start_line = edit.range.start.line as usize;
        let end_line = edit.range.end.line as usize;
        let start_char = edit.range.start.character as usize;
        let end_char = edit.range.end.character as usize;

        if start_line >= lines.len() {
            continue;
        }

        let end_line = end_line.min(lines.len() - 1);

        let prefix = &lines[start_line][..start_char.min(lines[start_line].len())];
        let suffix = &lines[end_line][end_char.min(lines[end_line].len())..];
        let new_text = format!("{prefix}{}{suffix}", edit.new_text);

        let new_lines: Vec<String> = new_text.lines().map(String::from).collect();
        lines.splice(start_line..=end_line, new_lines);
    }

    let result = lines.join("\n");
    std::fs::write(&path, &result)
        .map_err(|e| LspClientError::Other(format!("write error: {e}")))?;

    Ok(())
}
