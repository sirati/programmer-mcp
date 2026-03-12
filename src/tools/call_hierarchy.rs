//! Call hierarchy: find callers (incoming) and callees (outgoing) of a symbol.

use std::fmt::Write;
use std::sync::Arc;

use lsp_types::Position;

use crate::lsp::client::{LspClient, LspClientError};
use crate::tools::formatting::{path_to_uri, uri_to_path};
use crate::tools::symbol_search::find_symbol_with_fallback;

/// Find all callers of a symbol.
pub async fn callers(
    client: Arc<LspClient>,
    name: String,
    search_dir: Option<String>,
) -> Result<String, LspClientError> {
    let items = prepare_from_symbol(&client, &name, search_dir.as_deref()).await?;
    if items.is_empty() {
        return Ok(String::new());
    }

    let mut out = String::new();
    for item in items {
        let incoming = client.call_hierarchy_incoming(item).await?;
        let calls = incoming.unwrap_or_default();
        if calls.is_empty() {
            writeln!(out, "{name}: no callers found").ok();
            continue;
        }
        writeln!(out, "Callers of {name}:").ok();
        for call in &calls {
            let from = &call.from;
            let file = uri_to_path(&from.uri).unwrap_or_else(|| from.uri.as_str().to_string());
            let line = from.selection_range.start.line + 1;
            let detail = from.detail.as_deref().unwrap_or("");
            if detail.is_empty() {
                writeln!(out, "  {} ({}:L{})", from.name, file, line).ok();
            } else {
                writeln!(out, "  {} — {} ({}:L{})", from.name, detail, file, line).ok();
            }
        }
    }

    Ok(out.trim_end().to_string())
}

/// Find all callees from a symbol.
pub async fn callees(
    client: Arc<LspClient>,
    name: String,
    search_dir: Option<String>,
) -> Result<String, LspClientError> {
    let items = prepare_from_symbol(&client, &name, search_dir.as_deref()).await?;
    if items.is_empty() {
        return Ok(String::new());
    }

    let mut out = String::new();
    for item in items {
        let outgoing = client.call_hierarchy_outgoing(item).await?;
        let calls = outgoing.unwrap_or_default();
        if calls.is_empty() {
            writeln!(out, "{name}: no callees found").ok();
            continue;
        }
        writeln!(out, "Callees of {name}:").ok();
        for call in &calls {
            let to = &call.to;
            let file = uri_to_path(&to.uri).unwrap_or_else(|| to.uri.as_str().to_string());
            let line = to.selection_range.start.line + 1;
            let detail = to.detail.as_deref().unwrap_or("");
            if detail.is_empty() {
                writeln!(out, "  {} ({}:L{})", to.name, file, line).ok();
            } else {
                writeln!(out, "  {} — {} ({}:L{})", to.name, detail, file, line).ok();
            }
        }
    }

    Ok(out.trim_end().to_string())
}

/// Resolve a symbol name to a CallHierarchyItem via find_symbol_with_fallback + prepare.
async fn prepare_from_symbol(
    client: &Arc<LspClient>,
    name: &str,
    search_dir: Option<&str>,
) -> Result<Vec<lsp_types::CallHierarchyItem>, LspClientError> {
    let symbols = find_symbol_with_fallback(client, name, search_dir).await?;
    if symbols.is_empty() {
        return Ok(vec![]);
    }

    // Use the first match
    let sym = &symbols[0];
    let uri = &sym.location.uri;
    let pos = Position {
        line: sym.location.range.start.line,
        character: sym.location.range.start.character,
    };

    // Ensure file is open
    if let Some(path) = uri_to_path(uri) {
        let _ = client.open_file(&path).await;
    }

    let items = client.call_hierarchy_prepare(uri, pos).await?;
    Ok(items.unwrap_or_default())
}
