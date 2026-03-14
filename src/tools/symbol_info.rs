use std::collections::HashSet;
use std::sync::Arc;

use tracing::debug;

use super::formatting::{
    find_containing_symbol_range, read_range_from_file, relative_to, uri_to_path,
};
use super::symbol_search::find_symbol_with_fallback;
use crate::lsp::client::{LspClient, LspClientError};

/// Format a "not found" message with fuzzy suggestions and seeding notice.
pub async fn not_found_msg(client: &LspClient, symbol_name: &str) -> String {
    let mut msg = format!("{symbol_name} not found");
    if client.symbol_cache().is_seeding() {
        msg.push_str(" (index incomplete — still seeding)");
    }
    // Suggest similar symbols via fuzzy search
    let suggestions = client.symbol_cache().fuzzy_search(symbol_name, 5).await;
    if !suggestions.is_empty() {
        msg.push_str("\nDid you mean: ");
        let names: Vec<&str> = suggestions.iter().map(|s| s.name.as_str()).collect();
        msg.push_str(&names.join(", "));
    }
    msg
}

/// Extract the doc comment above a symbol's definition.
pub async fn get_docstring(
    client: &Arc<LspClient>,
    symbol_name: &str,
    search_dir: Option<&str>,
) -> Result<String, LspClientError> {
    let symbols = find_symbol_with_fallback(client, symbol_name, search_dir).await?;
    if symbols.is_empty() {
        return Ok(not_found_msg(client, symbol_name).await);
    }

    let mut results = Vec::new();
    let mut seen = HashSet::new();
    for sym in &symbols {
        let key = (
            sym.location.uri.as_str().to_string(),
            sym.location.range.start.line,
        );
        if !seen.insert(key) {
            continue;
        }
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

/// Extract the body (source code) of a symbol, using full definition range.
/// Returns raw source code without line number decoration, suitable for editing.
pub async fn get_body(
    client: &Arc<LspClient>,
    symbol_name: &str,
    search_dir: Option<&str>,
) -> Result<String, LspClientError> {
    let symbols = find_symbol_with_fallback(client, symbol_name, search_dir).await?;
    if symbols.is_empty() {
        return Ok(not_found_msg(client, symbol_name).await);
    }

    let ws_root = client.workspace_root();
    let sym = &symbols[0];
    let loc = &sym.location;
    let path = uri_to_path(&loc.uri).unwrap_or_else(|| loc.uri.as_str().to_string());
    let rel_path = relative_to(&path, ws_root);

    // Open the file so the LSP tracks it
    if let Err(e) = client.open_file(&path).await {
        debug!("error opening file {path}: {e}");
        return Ok(format!("Error opening {rel_path}: {e}"));
    }

    // Get full range via document symbols (same as definition does)
    let full_range = find_containing_symbol_range(client, &loc.uri, loc.range.start).await;
    let range = full_range.unwrap_or(loc.range);

    match read_range_from_file(&loc.uri, &range) {
        Ok(text) => {
            let header = format!(
                "# {rel_path} L{}-L{}\n",
                range.start.line + 1,
                range.end.line + 1,
            );
            Ok(format!("{header}{text}"))
        }
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
