use std::sync::Arc;

use lsp_types::{HoverContents, MarkedString};

use super::formatting::{find_identifier_position, path_to_uri, to_lsp_position, uri_to_path};
use super::language_specific;
use super::symbol_search::find_symbol_with_fallback;
use crate::lsp::client::{LspClient, LspClientError};

/// Get hover information at a specific position in a file.
pub async fn get_hover_info(
    client: &Arc<LspClient>,
    file_path: &str,
    line: u32,
    column: u32,
) -> Result<String, LspClientError> {
    client.open_file(file_path).await?;

    let uri = path_to_uri(file_path).map_err(LspClientError::Other)?;
    let position = to_lsp_position(line, column);

    let hover = client.hover(&uri, position).await?;

    match hover {
        Some(h) => Ok(clean_hover_text(
            client.language(),
            &format_hover_contents(&h.contents),
        )),
        None => Ok(format!(
            "No hover information available at {file_path}:{line}:{column}"
        )),
    }
}

fn format_hover_contents(contents: &HoverContents) -> String {
    match contents {
        HoverContents::Scalar(marked) => format_marked_string(marked),
        HoverContents::Array(items) => items
            .iter()
            .map(format_marked_string)
            .collect::<Vec<_>>()
            .join("\n\n"),
        HoverContents::Markup(markup) => markup.value.clone(),
    }
}

fn format_marked_string(s: &MarkedString) -> String {
    match s {
        MarkedString::String(text) => text.clone(),
        MarkedString::LanguageString(ls) => format!("```{}\n{}\n```", ls.language, ls.value),
    }
}

/// Get hover information for a symbol by name (resolves position automatically).
pub async fn hover_symbol(
    client: &Arc<LspClient>,
    symbol_name: &str,
    search_dir: Option<&str>,
) -> Result<String, LspClientError> {
    let symbols = find_symbol_with_fallback(client, symbol_name, search_dir).await?;
    if symbols.is_empty() {
        return Ok(format!("{symbol_name} not found"));
    }

    let sym = &symbols[0];
    let path =
        uri_to_path(&sym.location.uri).unwrap_or_else(|| sym.location.uri.as_str().to_string());
    client.open_file(&path).await?;

    let pos = find_identifier_position(&path, &sym.name, sym.location.range.start);
    let hover = client.hover(&sym.location.uri, pos).await?;

    match hover {
        Some(h) => Ok(clean_hover_text(
            client.language(),
            &format_hover_contents(&h.contents),
        )),
        None => Ok(format!("No hover information for {symbol_name}")),
    }
}

/// Strip LSP noise from hover text using language-specific rules.
fn clean_hover_text(language: &str, text: &str) -> String {
    if let Some(keyword) = language_specific::detect_keyword_doc(language, text) {
        return keyword.to_string();
    }

    let lines: Vec<&str> = text.lines().collect();
    let mut out = Vec::new();
    let mut in_code_block = false;
    for line in &lines {
        let trimmed = line.trim();
        if trimmed.starts_with("```") {
            in_code_block = !in_code_block;
        }
        if language_specific::is_noise_line(language, line) {
            continue;
        }
        // Outside code blocks, stop at headings after initial content
        if !in_code_block && trimmed.starts_with("# ") && out.len() > 3 {
            break;
        }
        out.push(*line);
        if out.len() >= 20 && !in_code_block {
            break;
        }
    }
    while out.last().is_some_and(|l| l.trim().is_empty()) {
        out.pop();
    }
    out.join("\n")
}
