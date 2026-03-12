//! Compact diagnostic output formatting.
//!
//! Groups pending diagnostics by directory, file, severity, and message type
//! to produce concise output with relative paths.

use std::collections::{BTreeMap, HashMap};
use std::path::Path;

use super::{DiagEntry, PendingDiag};

/// Format pending diagnostics in compact grouped form.
///
/// Output example:
/// ```text
/// New diagnostics based on recent edits:
/// cd src/lsp/client
/// 2 new warnings for mod.rs:
/// use of deprecated field:
///   L119:13 `...root_uri`: Use `workspace_folders` instead
///   L120:13 `...root_path`: Use `root_uri` instead
/// 1 new warning for mod.rs:
/// unused import:
///   L164:25 `futures::StreamExt`
/// ```
pub fn format_pending(workspace_root: &Path, items: Vec<PendingDiag>) -> String {
    let mut by_dir: BTreeMap<String, BTreeMap<String, Vec<DiagEntry>>> = BTreeMap::new();

    for item in items {
        let rel = make_relative(workspace_root, &item.file_path);
        let (dir, file) = split_dir_file(&rel);
        by_dir
            .entry(dir)
            .or_default()
            .entry(file)
            .or_default()
            .push(item.entry);
    }

    let mut out = Vec::new();
    out.push("New diagnostics based on recent edits:".to_string());

    for (dir, files) in &by_dir {
        if !dir.is_empty() {
            out.push(format!("cd {dir}"));
        }

        for (file, entries) in files {
            let grouped = group_by_severity_and_type(entries);

            for (severity, type_groups) in &grouped {
                let total: usize = type_groups.values().map(|v| v.len()).sum();
                let sev_word = if total == 1 {
                    severity.to_string()
                } else {
                    format!("{severity}s")
                };
                out.push(format!("{total} new {sev_word} for {file}:"));

                for (msg_type, locations) in type_groups {
                    out.push(format!("{msg_type}:"));
                    for (line, col, detail) in locations {
                        if detail.is_empty() {
                            out.push(format!("  L{line}:{col}"));
                        } else {
                            out.push(format!("  L{line}:{col} {detail}"));
                        }
                    }
                }
            }
        }
    }

    out.join("\n")
}

/// Make path relative to workspace root.
fn make_relative(workspace_root: &Path, abs_path: &str) -> String {
    let p = Path::new(abs_path);
    p.strip_prefix(workspace_root)
        .unwrap_or(p)
        .display()
        .to_string()
}

/// Split "src/lsp/client/mod.rs" into ("src/lsp/client", "mod.rs").
fn split_dir_file(rel_path: &str) -> (String, String) {
    let p = Path::new(rel_path);
    let file = p
        .file_name()
        .map(|f| f.to_string_lossy().to_string())
        .unwrap_or_else(|| rel_path.to_string());
    let dir = p
        .parent()
        .map(|d| d.display().to_string())
        .unwrap_or_default();
    (dir, file)
}

/// Group diagnostics by severity, then by message type.
/// Returns: Vec<(severity, BTreeMap<type_label, Vec<(line, col, detail)>>)>
/// Severities ordered: error, warning, info, hint, then rest.
fn group_by_severity_and_type(
    entries: &[DiagEntry],
) -> Vec<(String, BTreeMap<String, Vec<(u32, u32, String)>>)> {
    let mut by_sev: HashMap<String, HashMap<String, Vec<(u32, u32, String)>>> = HashMap::new();

    for e in entries {
        let (type_label, detail) = split_message_type(&e.message);
        by_sev
            .entry(e.severity.clone())
            .or_default()
            .entry(type_label)
            .or_default()
            .push((e.line, e.col, detail));
    }

    let mut result = Vec::new();
    let severity_order = ["error", "warning", "info", "hint", "diagnostic"];

    for sev in &severity_order {
        if let Some(types) = by_sev.remove(*sev) {
            push_sorted_types(&mut result, sev.to_string(), types);
        }
    }
    for (sev, types) in by_sev {
        push_sorted_types(&mut result, sev, types);
    }

    result
}

fn push_sorted_types(
    result: &mut Vec<(String, BTreeMap<String, Vec<(u32, u32, String)>>)>,
    severity: String,
    types: HashMap<String, Vec<(u32, u32, String)>>,
) {
    let mut sorted: BTreeMap<String, Vec<(u32, u32, String)>> = BTreeMap::new();
    for (label, mut locs) in types {
        locs.sort_by_key(|(l, c, _)| (*l, *c));
        sorted.insert(label, locs);
    }
    if !sorted.is_empty() {
        result.push((severity, sorted));
    }
}

/// Split a diagnostic message into a type label and case-specific detail.
///
/// Heuristic: split at the first backtick (common for rustc messages like
/// `"use of deprecated field `root_uri`"`), or at `": "` for other patterns.
fn split_message_type(message: &str) -> (String, String) {
    if let Some(pos) = message.find('`') {
        let label = message[..pos].trim().trim_end_matches(':').trim();
        let detail = message[pos..].trim().to_string();
        if !label.is_empty() {
            return (label.to_string(), detail);
        }
    }
    if let Some(pos) = message.find(": ") {
        let label = message[..pos].trim();
        let detail = message[pos + 2..].trim().to_string();
        if !label.is_empty() && label.len() < 60 {
            return (label.to_string(), detail);
        }
    }
    (message.to_string(), String::new())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_split_message_type_backtick() {
        let (label, detail) = split_message_type(
            "use of deprecated field `lsp_types::InitializeParams::root_uri`: Use `workspace_folders` instead",
        );
        assert_eq!(label, "use of deprecated field");
        assert!(detail.starts_with('`'));
    }

    #[test]
    fn test_split_message_type_colon() {
        let (label, detail) = split_message_type("unused import: `futures::StreamExt`");
        assert_eq!(label, "unused import");
        assert_eq!(detail, "`futures::StreamExt`");
    }

    #[test]
    fn test_make_relative() {
        let ws = Path::new("/home/user/project");
        assert_eq!(
            make_relative(ws, "/home/user/project/src/main.rs"),
            "src/main.rs"
        );
    }

    #[test]
    fn test_format_pending_groups_by_dir() {
        let ws = Path::new("/project");
        let items = vec![
            PendingDiag {
                file_path: "/project/src/lsp/client/mod.rs".to_string(),
                entry: DiagEntry {
                    severity: "warning".to_string(),
                    line: 119,
                    col: 13,
                    message: "use of deprecated field `root_uri`: Use `workspace_folders`"
                        .to_string(),
                },
            },
            PendingDiag {
                file_path: "/project/src/lsp/client/mod.rs".to_string(),
                entry: DiagEntry {
                    severity: "warning".to_string(),
                    line: 120,
                    col: 13,
                    message: "use of deprecated field `root_path`: Use `root_uri`".to_string(),
                },
            },
            PendingDiag {
                file_path: "/project/src/lsp/client/mod.rs".to_string(),
                entry: DiagEntry {
                    severity: "warning".to_string(),
                    line: 164,
                    col: 25,
                    message: "unused import `futures::StreamExt`".to_string(),
                },
            },
        ];
        let result = format_pending(ws, items);
        assert!(result.contains("cd src/lsp/client"));
        assert!(result.contains("mod.rs"));
        assert!(result.contains("L119:13"));
        assert!(result.contains("L120:13"));
        assert!(!result.contains("/project/"));
    }
}
