//! Call hierarchy: find callers (incoming) and callees (outgoing) of a symbol.

use std::fmt::Write;
use std::sync::Arc;

use lsp_types::{CallHierarchyItem, Position};

use crate::lsp::client::{LspClient, LspClientError};
use crate::tools::formatting::uri_to_path;
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
            format_item(&mut out, &call.from);
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
            format_item(&mut out, &call.to);
        }
    }

    Ok(out.trim_end().to_string())
}

/// Format a CallHierarchyItem as a concise line.
fn format_item(out: &mut String, item: &CallHierarchyItem) {
    let file = uri_to_path(&item.uri).unwrap_or_else(|| item.uri.as_str().to_string());
    let line = item.selection_range.start.line + 1;

    // Use short form for external/stdlib paths
    let location = if is_external(&file) {
        format!("(external)")
    } else {
        format!("({}:L{})", file, line)
    };

    if let Some(detail) = &item.detail {
        if !detail.is_empty() {
            // Show first line of detail only
            let short = detail.lines().next().unwrap_or(detail);
            writeln!(out, "  {} — {} {}", item.name, short, location).ok();
            return;
        }
    }
    writeln!(out, "  {} {}", item.name, location).ok();
}

/// Check if a path is external (stdlib, cargo registry, nix store, etc.)
fn is_external(path: &str) -> bool {
    path.contains("/.cargo/registry/")
        || path.contains("/rustlib/src/")
        || path.contains("/nix/store/")
        || path.starts_with("/usr/")
}

/// Resolve a symbol name to a CallHierarchyItem via find_symbol_with_fallback + prepare.
async fn prepare_from_symbol(
    client: &Arc<LspClient>,
    name: &str,
    search_dir: Option<&str>,
) -> Result<Vec<CallHierarchyItem>, LspClientError> {
    let symbols = find_symbol_with_fallback(client, name, search_dir).await?;
    if symbols.is_empty() {
        return Ok(vec![]);
    }

    let sym = &symbols[0];
    let uri = &sym.location.uri;
    let pos = Position {
        line: sym.location.range.start.line,
        character: sym.location.range.start.character,
    };

    if let Some(path) = uri_to_path(uri) {
        let _ = client.open_file(&path).await;
    }

    let items = client.call_hierarchy_prepare(uri, pos).await?;
    Ok(items.unwrap_or_default())
}
