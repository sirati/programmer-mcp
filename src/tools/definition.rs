use std::fmt::Write;
use std::sync::Arc;

use tracing::debug;

use super::formatting::{
    add_line_numbers, find_containing_symbol_range, read_range_from_file, uri_to_path,
};
use super::symbol_info::not_found_msg;
use super::symbol_search::find_symbol_with_fallback;
use crate::lsp::client::{LspClient, LspClientError};

/// Read the definition of a symbol, returning formatted source code.
pub async fn read_definition(
    client: &Arc<LspClient>,
    symbol_name: &str,
    search_dir: Option<&str>,
) -> Result<String, LspClientError> {
    let symbols = find_symbol_with_fallback(client, symbol_name, search_dir).await?;

    if symbols.is_empty() {
        return Ok(not_found_msg(client, symbol_name));
    }

    let mut output = String::new();

    for symbol in &symbols {
        let loc = &symbol.location;
        let path = uri_to_path(&loc.uri).unwrap_or_else(|| loc.uri.as_str().to_string());

        // Open the file so the LSP tracks it
        if let Err(e) = client.open_file(&path).await {
            debug!("error opening file {path}: {e}");
            continue;
        }

        // Try to get full definition range via document symbols
        let full_range = find_containing_symbol_range(client, &loc.uri, loc.range.start).await;
        let range = full_range.unwrap_or(loc.range);

        let text = match read_range_from_file(&loc.uri, &range) {
            Ok(t) => t,
            Err(e) => {
                debug!("error reading range: {e}");
                continue;
            }
        };

        let kind = format!("Kind: {:?}\n", symbol.kind);
        let container = symbol
            .container_name
            .as_ref()
            .map(|c| format!("Container Name: {c}\n"))
            .unwrap_or_default();

        let _ = write!(
            output,
            "---\n\n\
             Symbol: {}\n\
             File: {path}\n\
             {kind}\
             {container}\
             Range: L{}:C{} - L{}:C{}\n\n\
             {}\n",
            symbol.name,
            range.start.line + 1,
            range.start.character + 1,
            range.end.line + 1,
            range.end.character + 1,
            add_line_numbers(&text, range.start.line + 1),
        );
    }

    if output.is_empty() {
        return Ok(not_found_msg(client, symbol_name));
    }

    Ok(output)
}
