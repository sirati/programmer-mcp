use std::collections::BTreeMap;
use std::fmt::Write;
use std::sync::Arc;

use super::formatting::{format_lines_with_gaps, lines_to_display, uri_to_path};
use super::symbol_search::find_symbol_with_fallback;
use crate::lsp::client::{LspClient, LspClientError};

/// Find all references to a symbol throughout the codebase.
pub async fn find_references(
    client: &Arc<LspClient>,
    symbol_name: &str,
    context_lines: usize,
    search_dir: Option<&str>,
) -> Result<String, LspClientError> {
    let symbols = find_symbol_with_fallback(client, symbol_name, search_dir).await?;

    if symbols.is_empty() {
        return Ok(format!("No references found for symbol: {symbol_name}"));
    }

    let mut all_output = String::new();

    for symbol in &symbols {
        let loc = &symbol.location;
        let path = uri_to_path(&loc.uri).unwrap_or_else(|| loc.uri.as_str().to_string());

        if let Err(e) = client.open_file(&path).await {
            tracing::debug!("error opening file: {e}");
            continue;
        }

        let refs = client
            .references(&loc.uri, loc.range.start, false)
            .await?
            .unwrap_or_default();

        if refs.is_empty() {
            continue;
        }

        // Group references by file URI string
        let mut by_file: BTreeMap<String, Vec<lsp_types::Location>> = BTreeMap::new();
        for r in &refs {
            by_file
                .entry(r.uri.as_str().to_string())
                .or_default()
                .push(r.clone());
        }

        for (uri_str, file_refs) in &by_file {
            let file_path = uri_str.strip_prefix("file://").unwrap_or(uri_str);

            let content = match std::fs::read_to_string(file_path) {
                Ok(c) => c,
                Err(e) => {
                    let _ = writeln!(all_output, "---\n\n{file_path}\nError: {e}");
                    continue;
                }
            };

            let lines: Vec<&str> = content.lines().collect();

            let loc_strings: Vec<String> = file_refs
                .iter()
                .map(|r| {
                    format!(
                        "L{}:C{}",
                        r.range.start.line + 1,
                        r.range.start.character + 1
                    )
                })
                .collect();

            let visible = lines_to_display(file_refs, lines.len(), context_lines);
            let formatted = format_lines_with_gaps(&lines, &visible);

            let _ = write!(
                all_output,
                "---\n\n{file_path}\nReferences in File: {}\nAt: {}\n\n{formatted}",
                file_refs.len(),
                loc_strings.join(", "),
            );
        }
    }

    if all_output.is_empty() {
        return Ok(format!("No references found for symbol: {symbol_name}"));
    }

    Ok(all_output)
}
