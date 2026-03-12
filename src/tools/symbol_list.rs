use std::collections::BTreeMap;
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
            format_flat_grouped(&mut out, &symbols);
        }
    }

    if out.is_empty() {
        Ok("No symbols found".to_string())
    } else {
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
            format_children_grouped(out, children, depth + 1, max_depth);
        }
    }
}

/// Group children by kind and print leaf symbols compactly on one line per kind.
/// Symbols with their own children are printed individually and recurse.
fn format_children_grouped(
    out: &mut String,
    children: &[lsp_types::DocumentSymbol],
    depth: usize,
    max_depth: usize,
) {
    let mut leaf_groups: BTreeMap<&'static str, Vec<(&lsp_types::DocumentSymbol, u32)>> =
        BTreeMap::new();
    let mut branches: Vec<&lsp_types::DocumentSymbol> = Vec::new();

    for child in children {
        let has_children = child
            .children
            .as_ref()
            .map(|c| !c.is_empty())
            .unwrap_or(false);

        if has_children && depth < max_depth {
            branches.push(child);
        } else {
            let kind = symbol_kind_str(child.kind);
            let line = child.selection_range.start.line + 1;
            leaf_groups.entry(kind).or_default().push((child, line));
        }
    }

    let indent = "  ".repeat(depth);

    // Print grouped leaf symbols: "  fields: x L10, y L12, z L15"
    for (kind, symbols) in &leaf_groups {
        let entries: Vec<String> = symbols
            .iter()
            .map(|(sym, line)| format!("{} L{line}", sym.name))
            .collect();

        if entries.len() == 1 {
            writeln!(out, "{indent}{kind}: {}", entries[0]).ok();
        } else {
            let plural = symbol_kind_plural(kind);
            writeln!(out, "{indent}{plural}: {}", entries.join(", ")).ok();
        }
    }

    // Print branches individually (they recurse)
    for child in branches {
        format_symbol_tree(out, child, depth, max_depth);
    }
}

fn symbol_kind_plural(singular: &str) -> &'static str {
    match singular {
        "fn" => "fns",
        "method" => "methods",
        "field" => "fields",
        "variant" => "variants",
        "const" => "consts",
        "var" => "vars",
        "prop" => "props",
        "class" => "classes",
        "struct" => "structs",
        "enum" => "enums",
        "iface" => "ifaces",
        "mod" => "mods",
        "tparam" => "tparams",
        "key" => "keys",
        "ctor" => "ctors",
        "sym" => "syms",
        "file" => "files",
        "ns" => "namespaces",
        "pkg" => "packages",
        "op" => "ops",
        "event" => "events",
        _ => "items",
    }
}

/// Format flat symbol list, reconstructing hierarchy from `container_name`.
///
/// Symbols with a `container_name` are grouped under their parent.
/// Top-level symbols (no container) are grouped by kind.
fn format_flat_grouped(out: &mut String, symbols: &[lsp_types::SymbolInformation]) {
    // Build a name->(kind, line) lookup for all symbols
    let mut sym_info: Vec<(&str, &'static str, u32)> = Vec::new();
    for sym in symbols {
        let kind = symbol_kind_str(sym.kind);
        let line = sym.location.range.start.line + 1;
        sym_info.push((&sym.name, kind, line));
    }

    // Collect containers in insertion order (Vec instead of IndexMap)
    // Each entry: (container_name, kind_groups)
    let mut containers: Vec<(String, BTreeMap<&'static str, Vec<(&str, u32)>>)> = Vec::new();
    let mut top_level: BTreeMap<&'static str, Vec<(&str, u32)>> = BTreeMap::new();

    for sym in symbols {
        let kind = symbol_kind_str(sym.kind);
        let line = sym.location.range.start.line + 1;

        if let Some(container) = sym.container_name.as_deref().filter(|c| !c.is_empty()) {
            let pos = containers.iter().position(|(n, _)| n == container);
            let idx = match pos {
                Some(i) => i,
                None => {
                    containers.push((container.to_string(), BTreeMap::new()));
                    containers.len() - 1
                }
            };
            containers[idx]
                .1
                .entry(kind)
                .or_default()
                .push((&sym.name, line));
        } else {
            top_level.entry(kind).or_default().push((&sym.name, line));
        }
    }

    // Set of names that are containers
    let container_names: std::collections::HashSet<&str> =
        containers.iter().map(|(n, _)| n.as_str()).collect();

    // Print top-level symbols grouped by kind.
    // If a symbol has children (is a container), print it individually with children indented.
    let mut printed_containers = std::collections::HashSet::new();

    for (kind, syms) in &top_level {
        let mut parents = Vec::new();
        let mut leaves = Vec::new();
        for &(name, line) in syms {
            if container_names.contains(name) {
                parents.push((name, line));
            } else {
                leaves.push((name, line));
            }
        }

        for (name, line) in &parents {
            writeln!(out, "{kind} {name} L{line}").ok();
            printed_containers.insert(name.to_string());
            if let Some((_, groups)) = containers.iter().find(|(n, _)| n == *name) {
                write_grouped_children(out, groups, 1);
            }
        }

        if !leaves.is_empty() {
            let entries: Vec<String> = leaves
                .iter()
                .map(|(name, line)| format!("{name} L{line}"))
                .collect();
            if entries.len() == 1 {
                writeln!(out, "{kind}: {}", entries[0]).ok();
            } else {
                let plural = symbol_kind_plural(kind);
                writeln!(out, "{plural}: {}", entries.join(", ")).ok();
            }
        }
    }

    // Print containers that weren't in top_level (nested or orphan containers)
    let _top_names: std::collections::HashSet<&str> = top_level
        .values()
        .flat_map(|v| v.iter().map(|(n, _)| *n))
        .collect();
    for (name, groups) in &containers {
        if printed_containers.contains(name.as_str()) {
            continue;
        }
        // Look up kind/line from sym_info
        if let Some(&(_, kind, line)) = sym_info.iter().find(|(n, _, _)| *n == name.as_str()) {
            writeln!(out, "{kind} {name} L{line}").ok();
        } else {
            writeln!(out, "{name}:").ok();
        }
        write_grouped_children(out, groups, 1);
    }
}

/// Write grouped children at a given indent depth.
fn write_grouped_children(
    out: &mut String,
    groups: &BTreeMap<&'static str, Vec<(&str, u32)>>,
    depth: usize,
) {
    let indent = "  ".repeat(depth);
    for (kind, syms) in groups {
        let entries: Vec<String> = syms
            .iter()
            .map(|(name, line)| format!("{name} L{line}"))
            .collect();
        if entries.len() == 1 {
            writeln!(out, "{indent}{kind}: {}", entries[0]).ok();
        } else {
            let plural = symbol_kind_plural(kind);
            writeln!(out, "{indent}{plural}: {}", entries.join(", ")).ok();
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
