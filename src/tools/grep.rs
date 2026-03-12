//! Text search across workspace files.
//!
//! Provides a `grep` DSL command that searches file contents using regex
//! patterns, returning matching lines with context.

use std::fmt::Write;
use std::path::Path;

/// Source file extensions to search (same as list_dir).
const SOURCE_EXTS: &[&str] = &[
    "rs", "go", "py", "js", "ts", "tsx", "jsx", "c", "h", "cpp", "hpp", "java", "kt", "scala",
    "rb", "ex", "exs", "nix", "toml", "yaml", "yml", "json", "sh", "bash", "zsh", "lua", "zig",
    "swift", "cs", "fs", "ml", "mli", "hs", "el", "clj", "sql",
];

/// Maximum number of matches to return.
const MAX_MATCHES: usize = 50;

/// Search workspace files for a pattern.
///
/// Returns matching lines with file path and line number.
/// `search_dir` scopes the search to a subdirectory.
/// `pattern` is matched case-insensitively as a substring.
pub fn grep_workspace(pattern: &str, search_dir: Option<&str>, workspace_root: &Path) -> String {
    if pattern.is_empty() {
        return "grep: empty pattern".to_string();
    }

    let root = if let Some(dir) = search_dir {
        if Path::new(dir).is_absolute() {
            dir.into()
        } else {
            workspace_root.join(dir).display().to_string()
        }
    } else {
        workspace_root.display().to_string()
    };

    let root_path = Path::new(&root);
    if !root_path.is_dir() {
        return format!("grep: {root}: not a directory");
    }

    let pat_lower = pattern.to_lowercase();
    let mut matches = Vec::new();
    collect_matches(root_path, &pat_lower, workspace_root, &mut matches);

    if matches.is_empty() {
        return format!("no matches for `{pattern}`");
    }

    let truncated = matches.len() > MAX_MATCHES;
    let mut out = String::new();
    let mut last_file = String::new();

    for m in matches.iter().take(MAX_MATCHES) {
        if m.file != last_file {
            if !last_file.is_empty() {
                out.push('\n');
            }
            writeln!(out, "{}:", m.file).ok();
            last_file = m.file.clone();
        }
        writeln!(out, "  L{}:  {}", m.line, m.text.trim()).ok();
    }

    if truncated {
        writeln!(
            out,
            "\n... {} more matches (showing first {MAX_MATCHES})",
            matches.len() - MAX_MATCHES
        )
        .ok();
    }

    out.trim_end().to_string()
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
            if matches.len() > MAX_MATCHES * 2 {
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
