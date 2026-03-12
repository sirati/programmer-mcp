//! Document symbol flattening and language-aware file collection.
//!
//! Utilities for converting nested `DocumentSymbolResponse` trees into flat
//! `SymbolInformation` vectors, and for collecting source files filtered by
//! language.

use std::path::{Path, PathBuf};

use lsp_types::{DocumentSymbolResponse, Location, SymbolInformation, Uri};

use super::SOURCE_EXTS;

// ── Document symbol flattening ──────────────────────────────────────────────

/// Flatten a `DocumentSymbolResponse` into a list of `SymbolInformation`,
/// preserving container names from nesting.
pub fn flatten_doc_symbols(response: &DocumentSymbolResponse, uri: &Uri) -> Vec<SymbolInformation> {
    let mut out = Vec::new();
    match response {
        DocumentSymbolResponse::Flat(symbols) => {
            out.extend(symbols.iter().cloned());
        }
        DocumentSymbolResponse::Nested(symbols) => {
            flatten_nested(symbols, uri, None, &mut out);
        }
    }
    out
}

fn flatten_nested(
    symbols: &[lsp_types::DocumentSymbol],
    uri: &Uri,
    container: Option<&str>,
    out: &mut Vec<SymbolInformation>,
) {
    for sym in symbols {
        #[allow(deprecated)]
        out.push(SymbolInformation {
            name: sym.name.clone(),
            kind: sym.kind,
            tags: sym.tags.clone(),
            deprecated: sym.deprecated.map(|_| false),
            location: Location {
                uri: uri.clone(),
                range: sym.selection_range,
            },
            container_name: container.map(str::to_string),
        });
        if let Some(children) = &sym.children {
            flatten_nested(children, uri, Some(&sym.name), out);
        }
    }
}

// ── Language-aware file collection ──────────────────────────────────────────

/// Collect source files under `workspace` that match the given language.
pub fn collect_language_files(workspace: &Path, language: &str) -> Vec<PathBuf> {
    let exts = language_extensions(language);
    let mut files = Vec::new();
    collect_files_recursive(workspace, &exts, &mut files);
    files
}

fn collect_files_recursive(dir: &Path, exts: &[&str], out: &mut Vec<PathBuf>) {
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return,
    };

    for entry in entries.flatten() {
        let path = entry.path();
        let fname = entry.file_name();
        let fname_str = fname.to_string_lossy();

        if fname_str.starts_with('.')
            || fname_str == "target"
            || fname_str == "node_modules"
            || fname_str == "__pycache__"
            || fname_str == "venv"
            || fname_str == ".venv"
        {
            continue;
        }

        if path.is_dir() {
            collect_files_recursive(&path, exts, out);
        } else if path.is_file() {
            let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
            if exts.contains(&ext) {
                out.push(path);
            }
        }
    }
}

/// Map a language identifier to its file extensions.
fn language_extensions(language: &str) -> Vec<&'static str> {
    match language {
        "python" => vec!["py"],
        "rust" => vec!["rs"],
        "go" => vec!["go"],
        "javascript" => vec!["js", "jsx"],
        "typescript" => vec!["ts", "tsx"],
        "nix" => vec!["nix"],
        "c" | "cpp" => vec!["c", "cc", "cpp", "h", "hpp"],
        "java" => vec!["java"],
        "ruby" => vec!["rb"],
        "lua" => vec!["lua"],
        _ => SOURCE_EXTS.to_vec(),
    }
}
