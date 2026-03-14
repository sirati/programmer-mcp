use std::collections::BTreeSet;
use std::fmt::Write;
use std::path::Path;

use std::sync::Arc;

use lsp_types::{DocumentSymbolResponse, Location, Position, Range, Uri};

use crate::lsp::client::LspClient;

/// Convert a file path to an LSP `file://` URI.
pub fn path_to_uri(path: &str) -> Result<Uri, String> {
    let abs = if Path::new(path).is_absolute() {
        path.to_string()
    } else {
        std::env::current_dir()
            .map_err(|e| e.to_string())?
            .join(path)
            .to_string_lossy()
            .into()
    };
    let uri_str = format!("file://{abs}");
    uri_str
        .parse::<Uri>()
        .map_err(|e| format!("URI parse error: {e}"))
}

/// Extract the file path from a `file://` URI.
pub fn uri_to_path(uri: &Uri) -> Option<String> {
    let s = uri.as_str();
    s.strip_prefix("file://").map(|p| p.to_string())
}

/// Check if a path is external (stdlib, package registry, nix store, etc.)
pub fn is_external_path(path: &str) -> bool {
    path.contains("/.cargo/registry/")
        || path.contains("/rustlib/src/")
        || path.contains("/nix/store/")
        || path.contains("/go/pkg/mod/")
        || path.contains("/node_modules/")
        || path.contains("/site-packages/")
        || path.starts_with("/usr/")
}

/// Make an absolute path relative to a workspace root.
/// Returns the relative path, or the original path if it's not under root.
pub fn relative_to(path: &str, workspace_root: &Path) -> String {
    Path::new(path)
        .strip_prefix(workspace_root)
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_else(|_| path.to_string())
}

/// Find the full range of the symbol containing `position` via documentSymbol.
pub async fn find_containing_symbol_range(
    client: &Arc<LspClient>,
    uri: &Uri,
    position: Position,
) -> Option<Range> {
    let doc_symbols = client.document_symbol(uri).await.ok()?;

    match doc_symbols {
        DocumentSymbolResponse::Flat(symbols) => {
            // Find the smallest (most specific) range that contains `position`.
            // This avoids returning a broad container like `impl Foo { … }` when
            // a more specific child (e.g. a method) is available.
            symbols
                .iter()
                .filter(|s| contains_position(&s.location.range, position))
                .min_by_key(|s| {
                    let r = &s.location.range;
                    let lines = r.end.line.saturating_sub(r.start.line);
                    let chars = if r.start.line == r.end.line {
                        r.end.character.saturating_sub(r.start.character)
                    } else {
                        r.end.character
                    };
                    (lines, chars)
                })
                .map(|s| s.location.range)
        }
        DocumentSymbolResponse::Nested(symbols) => find_in_nested(&symbols, position),
    }
}

fn find_in_nested(symbols: &[lsp_types::DocumentSymbol], position: Position) -> Option<Range> {
    for sym in symbols {
        if contains_position(&sym.range, position) {
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

/// Compute which lines to display given a set of locations and context.
pub fn lines_to_display(
    locations: &[Location],
    total_lines: usize,
    context_lines: usize,
) -> BTreeSet<usize> {
    let mut lines = BTreeSet::new();

    for loc in locations {
        let ref_line = loc.range.start.line as usize;
        let start = ref_line.saturating_sub(context_lines);
        let end = (ref_line + context_lines).min(total_lines.saturating_sub(1));
        for i in start..=end {
            lines.insert(i);
        }
    }

    lines
}

/// Format lines with optional gaps shown as "...".
pub fn format_lines_with_gaps(all_lines: &[&str], visible: &BTreeSet<usize>) -> String {
    let mut result = String::new();
    let padding = all_lines.len().to_string().len();
    let mut last_line: Option<usize> = None;

    for &i in visible {
        if i >= all_lines.len() {
            continue;
        }
        if let Some(prev) = last_line {
            if i > prev + 1 {
                result.push_str("...\n");
            }
        }
        let num = i + 1; // 1-indexed
        let _ = writeln!(result, "{num:>padding$}|{}", all_lines[i]);
        last_line = Some(i);
    }

    result
}

/// Convert 1-indexed line/column to LSP 0-indexed Position.
pub fn to_lsp_position(line: u32, column: u32) -> Position {
    Position {
        line: line.saturating_sub(1),
        character: column.saturating_sub(1),
    }
}

/// Get the full definition text for a symbol range from a file.
pub fn read_range_from_file(uri: &Uri, range: &Range) -> Result<String, std::io::Error> {
    let path = uri_to_path(uri).ok_or_else(|| std::io::Error::other("invalid URI"))?;
    let content = std::fs::read_to_string(&path)?;
    let lines: Vec<&str> = content.lines().collect();

    let start = range.start.line as usize;
    let end = (range.end.line as usize).min(lines.len().saturating_sub(1));

    if start >= lines.len() {
        return Ok(String::new());
    }

    Ok(lines[start..=end].join("\n"))
}

/// Find the actual identifier position in a file for a symbol.
///
/// `workspace/symbol` range start often points at a doc comment or attribute line,
/// not the identifier. We scan forward from the range start to find the line
/// containing the symbol name and return the position of the name on that line.
pub fn find_identifier_position(path: &str, symbol_name: &str, range_start: Position) -> Position {
    let Ok(content) = std::fs::read_to_string(path) else {
        return range_start;
    };
    let lines: Vec<&str> = content.lines().collect();
    let start = range_start.line as usize;

    // The bare name: for "MyStruct.method" use "method"
    let bare = symbol_name
        .rsplit_once('.')
        .map(|(_, n)| n)
        .unwrap_or(symbol_name);

    // Scan up to 30 lines forward from range start to find the identifier
    for i in start..lines.len().min(start + 30) {
        if let Some(col) = lines[i].find(bare) {
            return Position {
                line: i as u32,
                character: col as u32,
            };
        }
    }

    range_start
}
