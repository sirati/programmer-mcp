//! Directory listing for source files.
//!
//! When `list_symbols` is called on a directory, this module provides a
//! tree-style listing of source files within that directory.

use std::fmt::Write;
use std::path::Path;

use super::SOURCE_EXTS;

/// List source files in a directory as a tree.
///
/// Returns a compact listing grouped by subdirectory, showing only source
/// files (by extension). `max_depth` controls how deep to recurse.
pub fn list_source_files(dir_path: &str, max_depth: usize, workspace_root: &Path) -> String {
    let abs_dir = if Path::new(dir_path).is_absolute() {
        dir_path.into()
    } else {
        workspace_root.join(dir_path).display().to_string()
    };

    let abs = Path::new(&abs_dir);
    if !abs.is_dir() {
        return format!("{dir_path}: not a directory");
    }

    let mut out = String::new();
    format_dir_compact(&mut out, abs, abs, max_depth, 0);

    if out.is_empty() {
        return format!("{dir_path}: no source files found");
    }

    out.trim_end().to_string()
}

/// Compact directory listing: `dirs: a, b  files: c, d, e`
/// Recurses into subdirs with indentation.
fn format_dir_compact(out: &mut String, base: &Path, dir: &Path, max_depth: usize, depth: usize) {
    let Ok(read_dir) = std::fs::read_dir(dir) else {
        return;
    };

    let mut files: Vec<String> = Vec::new();
    let mut subdirs: Vec<(String, std::path::PathBuf)> = Vec::new();

    for entry in read_dir.flatten() {
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().to_string();

        if name.starts_with('.')
            || name == "target"
            || name == "node_modules"
            || name == "__pycache__"
        {
            continue;
        }

        if path.is_dir() {
            subdirs.push((name, path));
        } else if is_source_file(&name) {
            files.push(name);
        }
    }

    files.sort();
    subdirs.sort_by(|a, b| a.0.cmp(&b.0));

    let indent = "  ".repeat(depth);

    // Print dirs line
    if !subdirs.is_empty() {
        let dir_names: Vec<&str> = subdirs.iter().map(|(n, _)| n.as_str()).collect();
        if dir_names.len() == 1 {
            writeln!(out, "{indent}dir: {}", dir_names[0]).ok();
        } else {
            writeln!(out, "{indent}dirs: {}", dir_names.join(", ")).ok();
        }
    }

    // Print files line
    if !files.is_empty() {
        if files.len() == 1 {
            writeln!(out, "{indent}file: {}", files[0]).ok();
        } else {
            writeln!(out, "{indent}files: {}", files.join(", ")).ok();
        }
    }

    // Recurse into subdirs
    if depth < max_depth {
        for (name, path) in &subdirs {
            writeln!(out, "{indent}{name}/").ok();
            format_dir_compact(out, base, path, max_depth, depth + 1);
        }
    }
}

fn is_source_file(name: &str) -> bool {
    if let Some(ext) = Path::new(name).extension().and_then(|e| e.to_str()) {
        SOURCE_EXTS.contains(&ext)
    } else {
        // Include extensionless files that look like scripts (Makefile, Dockerfile, etc.)
        matches!(
            name,
            "Makefile" | "Dockerfile" | "Justfile" | "Rakefile" | "Gemfile" | "Cargo.lock"
        )
    }
}
