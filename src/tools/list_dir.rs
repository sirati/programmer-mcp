//! Directory listing for source files.
//!
//! When `list_symbols` is called on a directory, this module provides a
//! tree-style listing of source files within that directory.

use std::collections::BTreeMap;
use std::fmt::Write;
use std::path::Path;

/// Source file extensions we consider relevant.
const SOURCE_EXTS: &[&str] = &[
    "rs", "go", "py", "js", "ts", "tsx", "jsx", "c", "h", "cpp", "hpp", "java", "kt", "scala",
    "rb", "ex", "exs", "nix", "toml", "yaml", "yml", "json", "sh", "bash", "zsh", "lua", "zig",
    "swift", "cs", "fs", "ml", "mli", "hs", "el", "clj", "sql",
];

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

    let mut entries: BTreeMap<String, Vec<String>> = BTreeMap::new();
    collect_files(abs, abs, max_depth, 0, &mut entries);

    if entries.is_empty() {
        return format!("{dir_path}: no source files found");
    }

    let mut out = String::new();
    for (subdir, files) in &entries {
        if subdir.is_empty() {
            // Files at root level
            for f in files {
                writeln!(out, "{f}").ok();
            }
        } else {
            writeln!(out, "{subdir}/").ok();
            for f in files {
                writeln!(out, "  {f}").ok();
            }
        }
    }

    out.trim_end().to_string()
}

fn collect_files(
    base: &Path,
    dir: &Path,
    max_depth: usize,
    depth: usize,
    entries: &mut BTreeMap<String, Vec<String>>,
) {
    let Ok(read_dir) = std::fs::read_dir(dir) else {
        return;
    };

    let rel_dir = dir
        .strip_prefix(base)
        .map(|p| p.display().to_string())
        .unwrap_or_default();

    let mut files: Vec<String> = Vec::new();
    let mut subdirs: Vec<std::path::PathBuf> = Vec::new();

    for entry in read_dir.flatten() {
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().to_string();

        // Skip hidden dirs and common non-source dirs
        if name.starts_with('.')
            || name == "target"
            || name == "node_modules"
            || name == "__pycache__"
        {
            continue;
        }

        if path.is_dir() {
            subdirs.push(path);
        } else if is_source_file(&name) {
            files.push(name);
        }
    }

    files.sort();
    subdirs.sort();

    if !files.is_empty() {
        entries.insert(rel_dir.clone(), files);
    }

    if depth < max_depth {
        for subdir in subdirs {
            collect_files(base, &subdir, max_depth, depth + 1, entries);
        }
    } else if !subdirs.is_empty() {
        // Show subdirectory names as hints at max depth
        let dir_names: Vec<String> = subdirs
            .iter()
            .filter_map(|d| d.file_name().map(|n| format!("{}/", n.to_string_lossy())))
            .collect();
        entries.entry(rel_dir).or_default().extend(dir_names);
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
