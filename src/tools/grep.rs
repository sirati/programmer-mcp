//! Text search across workspace files with symbol-aware output.
//!
//! The `grep` command searches file contents for a pattern, returning
//! results sorted with LSP-resolved symbol matches first, followed
//! by plain text matches.

use std::fmt::Write;
use std::path::Path;

use crate::lsp::manager::LspManager;

use super::formatting::relative_to;
use super::SOURCE_EXTS;

/// Maximum number of text matches to return.
const MAX_TEXT_MATCHES: usize = 50;
/// Maximum number of symbol matches to return.
const MAX_SYMBOL_MATCHES: usize = 20;

/// Search workspace for a pattern: symbols first, then text matches.
pub async fn grep_workspace(
    pattern: &str,
    search_dir: Option<&str>,
    workspace_root: &Path,
    manager: &LspManager,
) -> String {
    if pattern.is_empty() {
        return "grep: empty pattern".to_string();
    }

    let mut out = String::new();

    // 1. Symbol matches from the index
    let symbol_results = search_symbols(pattern, manager).await;
    if !symbol_results.is_empty() {
        let count = symbol_results.len().min(MAX_SYMBOL_MATCHES);
        writeln!(out, "Symbol matches ({count}):").ok();
        for sym_line in symbol_results.iter().take(MAX_SYMBOL_MATCHES) {
            writeln!(out, "  {sym_line}").ok();
        }
        if symbol_results.len() > MAX_SYMBOL_MATCHES {
            writeln!(
                out,
                "  ... {} more symbol matches",
                symbol_results.len() - MAX_SYMBOL_MATCHES
            )
            .ok();
        }
    }

    // 2. Text matches from files
    let root = resolve_search_root(search_dir, workspace_root);
    let root_path = Path::new(&root);
    if !root_path.is_dir() {
        if out.is_empty() {
            return format!("grep: {root}: not a directory");
        }
        return out.trim_end().to_string();
    }

    let pat_lower = pattern.to_lowercase();
    let mut matches = Vec::new();
    collect_matches(root_path, &pat_lower, workspace_root, &mut matches);

    if !matches.is_empty() {
        if !out.is_empty() {
            out.push('\n');
        }
        let total = matches.len();
        let showing = total.min(MAX_TEXT_MATCHES);
        writeln!(
            out,
            "Text matches ({showing}{}):",
            if total > showing {
                format!("/{total}")
            } else {
                String::new()
            }
        )
        .ok();

        let mut last_file = String::new();
        for m in matches.iter().take(MAX_TEXT_MATCHES) {
            if m.file != last_file {
                if !last_file.is_empty() {
                    out.push('\n');
                }
                writeln!(out, "  {}:", m.file).ok();
                last_file = m.file.clone();
            }
            writeln!(out, "    L{}:  {}", m.line, m.text.trim()).ok();
        }
    }

    if out.is_empty() {
        return format!("no matches for `{pattern}`");
    }

    out.trim_end().to_string()
}

/// Search symbol caches for pattern matches.
async fn search_symbols(pattern: &str, manager: &LspManager) -> Vec<String> {
    let clients = manager.resolve(None, None);
    let mut results = Vec::new();

    for client in &clients {
        let cache = client.symbol_cache();
        let ws_root = client.workspace_root();
        let lang = client.language();

        let matches = cache.fuzzy_search(pattern, MAX_SYMBOL_MATCHES).await;
        for sym in matches {
            let path = super::formatting::uri_to_path(&sym.location.uri).unwrap_or_default();
            let rel = relative_to(&path, ws_root);
            let line = sym.location.range.start.line + 1;
            let kind = format!("{:?}", sym.kind);
            let container = sym
                .container_name
                .as_deref()
                .map(|c| format!(" ({c})"))
                .unwrap_or_default();
            results.push(format!(
                "{rel}:{line}  {kind}  {}{container}  [{lang}]",
                sym.name
            ));
        }
    }

    results
}

fn resolve_search_root(search_dir: Option<&str>, workspace_root: &Path) -> String {
    if let Some(dir) = search_dir {
        if Path::new(dir).is_absolute() {
            dir.into()
        } else {
            workspace_root.join(dir).display().to_string()
        }
    } else {
        workspace_root.display().to_string()
    }
}

struct Match {
    file: String,
    line: usize,
    text: String,
}

fn collect_matches(dir: &Path, pattern: &str, workspace_root: &Path, matches: &mut Vec<Match>) {
    let Ok(read_dir) = std::fs::read_dir(dir) else {
        return;
    };

    let mut entries: Vec<_> = read_dir.flatten().collect();
    entries.sort_by_key(|e| e.file_name());

    for entry in entries {
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().to_string();

        // Skip hidden, target, node_modules
        if name.starts_with('.')
            || name == "target"
            || name == "node_modules"
            || name == "__pycache__"
        {
            continue;
        }

        if path.is_dir() {
            collect_matches(&path, pattern, workspace_root, matches);
            if matches.len() > MAX_TEXT_MATCHES * 2 {
                return; // Stop early if we have way too many
            }
        } else if is_searchable(&name) {
            search_file(&path, pattern, workspace_root, matches);
        }
    }
}

fn search_file(path: &Path, pattern: &str, workspace_root: &Path, matches: &mut Vec<Match>) {
    let Ok(content) = std::fs::read_to_string(path) else {
        return;
    };

    let rel = path
        .strip_prefix(workspace_root)
        .unwrap_or(path)
        .display()
        .to_string();

    for (i, line) in content.lines().enumerate() {
        if line.to_lowercase().contains(pattern) {
            matches.push(Match {
                file: rel.clone(),
                line: i + 1,
                text: line.to_string(),
            });
        }
    }
}

fn is_searchable(name: &str) -> bool {
    if let Some(ext) = Path::new(name).extension().and_then(|e| e.to_str()) {
        SOURCE_EXTS.contains(&ext)
    } else {
        matches!(
            name,
            "Makefile" | "Dockerfile" | "Justfile" | "Cargo.lock" | "Cargo.toml"
        )
    }
}
