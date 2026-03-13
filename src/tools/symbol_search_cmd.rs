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

    // Collect per-client results (exact first, then fuzzy, deduped).
    let mut per_client: Vec<Vec<(lsp_types::SymbolInformation, String)>> = Vec::new();

    for client in &clients {
        let mut results = Vec::new();
        let lang = client.language().to_string();

        let exact = client.symbol_cache().exact_search(query).await;
        for sym in exact {
            results.push((sym, lang.clone()));
        }

        let fuzzy = client.symbol_cache().fuzzy_search(query, limit).await;
        for sym in fuzzy {
            let already = results.iter().any(|(s, _)| {
                s.name == sym.name
                    && s.location.uri == sym.location.uri
                    && s.location.range.start.line == sym.location.range.start.line
            });
            if !already {
                results.push((sym, lang.clone()));
            }
        }

        if !results.is_empty() {
            per_client.push(results);
        }
    }

    if per_client.is_empty() {
        let mut msg = format!("no symbols matching '{query}'");
        let seeding: Vec<&str> = clients
            .iter()
            .filter(|c| c.symbol_cache().is_seeding())
            .map(|c| c.language())
            .collect();
        if !seeding.is_empty() {
            msg.push_str(&format!(
                " (index incomplete — still seeding: {})",
                seeding.join(", ")
            ));
        }
        return OperationResult {
            operation: "search".into(),
            success: true,
            output: msg,
        };
    }

    // Round-robin interleave so each language gets fair representation.
    let mut all_results = Vec::with_capacity(limit);
    let mut indices: Vec<usize> = vec![0; per_client.len()];
    loop {
        let mut added = false;
        for (i, client_results) in per_client.iter().enumerate() {
            if indices[i] < client_results.len() && all_results.len() < limit {
                all_results.push(client_results[indices[i]].clone());
                indices[i] += 1;
                added = true;
            }
        }
        if !added || all_results.len() >= limit {
            break;
        }
    }

    // Check if any client is still seeding.
    let seeding_langs: Vec<String> = clients
        .iter()
        .filter(|c| c.symbol_cache().is_seeding())
        .map(|c| c.language().to_string())
        .collect();

    let mut output = format!(
        "symbols matching '{query}' ({} results):\n",
        all_results.len()
    );
    if !seeding_langs.is_empty() {
        output.push_str(&format!(
            "  (index incomplete — still seeding: {})\n",
            seeding_langs.join(", ")
        ));
    }
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
