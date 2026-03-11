use std::fmt::Write;
use std::sync::Arc;

use lsp_types::{DocumentSymbolResponse, Position, Range, Uri};
use tracing::debug;

use super::formatting::{add_line_numbers, read_range_from_file, uri_to_path};
use super::symbol_search::find_symbol_with_fallback;
use crate::lsp::client::{LspClient, LspClientError};

/// Read the definition of a symbol, returning formatted source code.
pub async fn read_definition(
    client: &Arc<LspClient>,
    symbol_name: &str,
) -> Result<String, LspClientError> {
    let symbols = find_symbol_with_fallback(client, symbol_name).await?;

    if symbols.is_empty() {
        return Ok(format!("{symbol_name} not found"));
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
        return Ok(format!("{symbol_name} not found"));
    }

    Ok(output)
}

/// Find the full range of the symbol containing `position` via documentSymbol.
async fn find_containing_symbol_range(
    client: &Arc<LspClient>,
    uri: &Uri,
    position: Position,
) -> Option<Range> {
    let doc_symbols = client.document_symbol(uri).await.ok()?;

    match doc_symbols {
        DocumentSymbolResponse::Flat(symbols) => symbols
            .iter()
            .find(|s| contains_position(&s.location.range, position))
            .map(|s| s.location.range),
        DocumentSymbolResponse::Nested(symbols) => find_in_nested(&symbols, position),
    }
}

fn find_in_nested(symbols: &[lsp_types::DocumentSymbol], position: Position) -> Option<Range> {
    for sym in symbols {
        if contains_position(&sym.range, position) {
            // Check children for more specific match
            if let Some(children) = &sym.children {
                if let Some(child_range) = find_in_nested(children, position) {
                    return Some(child_range);
                }
            }
            return Some(sym.range);
        }
    }
    None
}

fn contains_position(range: &Range, pos: Position) -> bool {
    (range.start.line < pos.line
        || (range.start.line == pos.line && range.start.character <= pos.character))
        && (range.end.line > pos.line
            || (range.end.line == pos.line && range.end.character >= pos.character))
}
