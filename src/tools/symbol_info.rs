use std::sync::Arc;

use super::formatting::{read_range_from_file, uri_to_path};
use super::symbol_search::find_symbol_with_fallback;
use crate::lsp::client::{LspClient, LspClientError};

/// Extract the doc comment above a symbol's definition.
pub async fn get_docstring(
    client: &Arc<LspClient>,
    symbol_name: &str,
) -> Result<String, LspClientError> {
    let symbols = find_symbol_with_fallback(client, symbol_name).await?;
    if symbols.is_empty() {
        return Ok(format!("{symbol_name} not found"));
    }

    let mut results = Vec::new();
    for sym in &symbols {
        let path =
            uri_to_path(&sym.location.uri).unwrap_or_else(|| sym.location.uri.as_str().to_string());
        let start_line = sym.location.range.start.line as usize;
        let doc = extract_docstring_from_file(&path, start_line);
        if let Some(doc) = doc {
            results.push(format!("{}:\n{}", sym.name, doc));
        } else {
            results.push(format!("{}: (no docstring)", sym.name));
        }
    }
    Ok(results.join("\n\n"))
}

/// Extract the body (source code) of a symbol.
pub async fn get_body(
    client: &Arc<LspClient>,
    symbol_name: &str,
) -> Result<String, LspClientError> {
    let symbols = find_symbol_with_fallback(client, symbol_name).await?;
    if symbols.is_empty() {
        return Ok(format!("{symbol_name} not found"));
    }

    let sym = &symbols[0];
    let range = sym.location.range;
    match read_range_from_file(&sym.location.uri, &range) {
        Ok(text) => Ok(text),
        Err(e) => Ok(format!("Error reading body: {e}")),
    }
}

/// Read backwards from `start_line` to extract doc comments (/// or //! or #[doc...]).
fn extract_docstring_from_file(path: &str, start_line: usize) -> Option<String> {
    let content = std::fs::read_to_string(path).ok()?;
    let lines: Vec<&str> = content.lines().collect();

    if start_line == 0 || start_line > lines.len() {
        return None;
    }

    let mut doc_lines = Vec::new();
    let mut i = start_line.saturating_sub(1);
    loop {
        let trimmed = lines.get(i)?.trim();
        if trimmed.starts_with("///") || trimmed.starts_with("//!") || trimmed.starts_with("#[doc")
        {
            doc_lines.push(trimmed);
        } else if trimmed.is_empty() && !doc_lines.is_empty() {
            // Allow blank lines within doc blocks
        } else {
            break;
        }
        if i == 0 {
            break;
        }
        i -= 1;
    }

    if doc_lines.is_empty() {
        return None;
    }

    doc_lines.reverse();
    Some(doc_lines.join("\n"))
}
