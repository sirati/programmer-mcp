//! `search` command: fuzzy symbol search across LSP indices.

use std::fmt::Write;

use crate::lsp::manager::LspManager;
use crate::tools::formatting::uri_to_path;
use crate::tools::operation::OperationResult;

/// Execute a fuzzy symbol search across all (or language-filtered) LSP clients.
pub async fn execute_search_symbols(
    manager: &LspManager,
    query: &str,
    language: Option<&str>,
    limit: usize,
) -> OperationResult {
    let clients = manager.resolve(language, None);
    if clients.is_empty() {
        return OperationResult {
            operation: "search".into(),
            success: false,
            output: "no LSP clients available".into(),
        };
    }

    let mut all_results = Vec::new();

    for client in &clients {
        // Try exact search first
        let exact = client.symbol_cache().exact_search(query).await;
        for sym in exact {
            all_results.push((sym, client.language().to_string()));
        }

        // Then fuzzy search
        let fuzzy = client.symbol_cache().fuzzy_search(query, limit).await;
        for sym in fuzzy {
            // Deduplicate: skip if already in exact results
            let key = (
                sym.name.clone(),
                sym.location.uri.as_str().to_string(),
                sym.location.range.start.line,
            );
            let already = all_results.iter().any(|(s, _)| {
                s.name == key.0
                    && s.location.uri.as_str() == key.1
                    && s.location.range.start.line == key.2
            });
            if !already {
                all_results.push((sym, client.language().to_string()));
            }
        }
    }

    if all_results.is_empty() {
        return OperationResult {
            operation: "search".into(),
            success: true,
            output: format!("no symbols matching '{query}'"),
        };
    }

    // Truncate to limit
    all_results.truncate(limit);

    let mut output = format!(
        "symbols matching '{query}' ({} results):\n",
        all_results.len()
    );
    for (sym, lang) in &all_results {
        let path = uri_to_path(&sym.location.uri).unwrap_or_default();
        // Make path relative
        let rel = if let Ok(cwd) = std::env::current_dir() {
            let cwd_str = cwd.display().to_string();
            path.strip_prefix(&format!("{cwd_str}/"))
                .unwrap_or(&path)
                .to_string()
        } else {
            path
        };
        let line = sym.location.range.start.line + 1;
        let kind = format!("{:?}", sym.kind);
        let container = sym
            .container_name
            .as_deref()
            .map(|c| format!(" ({c})"))
            .unwrap_or_default();
        let _ = writeln!(
            output,
            "  {rel}:{line}  {kind}  {}{container}  [{lang}]",
            sym.name
        );
    }

    OperationResult {
        operation: "search".into(),
        success: true,
        output,
    }
}
