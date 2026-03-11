use std::fmt::Write;
use std::sync::Arc;

use lsp_types::{DocumentSymbolResponse, SymbolKind};

use super::formatting::path_to_uri;
use crate::lsp::client::{LspClient, LspClientError};

/// List symbols in a file as a tree, up to `max_depth` levels deep.
pub async fn list_symbols(
    client: &Arc<LspClient>,
    file_path: &str,
    max_depth: usize,
) -> Result<String, LspClientError> {
    client.open_file(file_path).await?;
    let uri = path_to_uri(file_path).map_err(LspClientError::Other)?;
    let response = client.document_symbol(&uri).await?;

    let mut out = String::new();
    match response {
        DocumentSymbolResponse::Nested(symbols) => {
            for sym in &symbols {
                format_symbol_tree(&mut out, sym, 0, max_depth);
            }
        }
        DocumentSymbolResponse::Flat(symbols) => {
            for sym in &symbols {
                let kind = symbol_kind_str(sym.kind);
                let line = sym.location.range.start.line + 1;
                writeln!(out, "{kind} {name} L{line}", name = sym.name).ok();
            }
        }
    }

    if out.is_empty() {
        Ok("No symbols found".to_string())
    } else {
        // Trim trailing newline
        Ok(out.trim_end().to_string())
    }
}

fn format_symbol_tree(
    out: &mut String,
    sym: &lsp_types::DocumentSymbol,
    depth: usize,
    max_depth: usize,
) {
    let indent = "  ".repeat(depth);
    let kind = symbol_kind_str(sym.kind);
    let line = sym.selection_range.start.line + 1;
    let detail = sym.detail.as_deref().unwrap_or("");
    if detail.is_empty() {
        writeln!(out, "{indent}{kind} {name} L{line}", name = sym.name).ok();
    } else {
        writeln!(
            out,
            "{indent}{kind} {name} L{line} — {detail}",
            name = sym.name
        )
        .ok();
    }

    if depth < max_depth {
        if let Some(children) = &sym.children {
            for child in children {
                format_symbol_tree(out, child, depth + 1, max_depth);
            }
        }
    }
}

fn symbol_kind_str(kind: SymbolKind) -> &'static str {
    match kind {
        SymbolKind::FILE => "file",
        SymbolKind::MODULE => "mod",
        SymbolKind::NAMESPACE => "ns",
        SymbolKind::PACKAGE => "pkg",
        SymbolKind::CLASS => "class",
        SymbolKind::METHOD => "method",
        SymbolKind::PROPERTY => "prop",
        SymbolKind::FIELD => "field",
        SymbolKind::CONSTRUCTOR => "ctor",
        SymbolKind::ENUM => "enum",
        SymbolKind::INTERFACE => "iface",
        SymbolKind::FUNCTION => "fn",
        SymbolKind::VARIABLE => "var",
        SymbolKind::CONSTANT => "const",
        SymbolKind::STRING => "str",
        SymbolKind::NUMBER => "num",
        SymbolKind::BOOLEAN => "bool",
        SymbolKind::ARRAY => "arr",
        SymbolKind::OBJECT => "obj",
        SymbolKind::KEY => "key",
        SymbolKind::NULL => "null",
        SymbolKind::ENUM_MEMBER => "variant",
        SymbolKind::STRUCT => "struct",
        SymbolKind::EVENT => "event",
        SymbolKind::OPERATOR => "op",
        SymbolKind::TYPE_PARAMETER => "tparam",
        _ => "sym",
    }
}
