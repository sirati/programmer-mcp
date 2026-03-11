use std::sync::Arc;

use lsp_types::{HoverContents, MarkedString};

use super::formatting::{path_to_uri, to_lsp_position};
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
        Some(h) => Ok(format_hover_contents(&h.contents)),
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
