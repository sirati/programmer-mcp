//! Workspace discovery — detect subprojects and standalone source files.

use std::collections::BTreeMap;
use std::fmt::Write;
use std::fs;
use std::path::Path;

use crate::lsp::detect_lang::detect_language_id;

/// Project markers and their human-readable type.
const PROJECT_MARKERS: &[(&str, &str)] = &[
    ("Cargo.toml", "rust/cargo"),
    ("go.mod", "go"),
    ("package.json", "node"),
    ("pyproject.toml", "python"),
    ("setup.py", "python"),
    ("build.gradle", "gradle"),
    ("build.gradle.kts", "gradle"),
    ("pom.xml", "maven"),
    ("CMakeLists.txt", "cmake"),
    ("Makefile", "make"),
    ("flake.nix", "nix"),
    ("dune-project", "ocaml/dune"),
    ("mix.exs", "elixir"),
    ("Gemfile", "ruby"),
    ("composer.json", "php"),
];

struct SubProject {
    path: String,
    kind: String,
    is_workspace: bool,
}

/// Scan the workspace and return a formatted description of subprojects and standalone files.
pub fn workspace_info(workspace_root: &Path) -> String {
    let mut subprojects = Vec::new();
    let mut standalone_by_dir: BTreeMap<String, Vec<String>> = BTreeMap::new();

    scan_recursive(
        workspace_root,
        workspace_root,
        &mut subprojects,
        &mut standalone_by_dir,
        false,
    );

    format_output(&subprojects, &standalone_by_dir)
}

/// Recursively scan directories for project markers and standalone source files.
/// `inside_project` is true when we're inside a directory that already has a project marker
/// — in that case we don't collect standalone files (they belong to the project).
fn scan_recursive(
    root: &Path,
    dir: &Path,
    projects: &mut Vec<SubProject>,
    standalone: &mut BTreeMap<String, Vec<String>>,
    inside_project: bool,
) {
    let rel = dir
        .strip_prefix(root)
        .unwrap_or(dir)
        .to_string_lossy()
        .to_string();
    let rel_display = if rel.is_empty() { ".".to_string() } else { rel };

    // Check this directory for project markers
    let is_project = check_dir_for_project(dir, &rel_display, projects);
    let in_project = inside_project || is_project;

    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    let mut entries: Vec<_> = entries.filter_map(|e| e.ok()).collect();
    entries.sort_by_key(|e| e.file_name());

    let mut subdirs = Vec::new();

    for entry in &entries {
        let name = entry.file_name().to_string_lossy().to_string();
        if is_hidden_or_ignored(&name) {
            continue;
        }
        let ft = match entry.file_type() {
            Ok(ft) => ft,
            Err(_) => continue,
        };
        if ft.is_dir() {
            subdirs.push(entry.path());
        } else if ft.is_file() && !in_project && is_source_file(&name) {
            standalone
                .entry(rel_display.clone())
                .or_default()
                .push(name);
        }
    }

    for subdir in subdirs {
        scan_recursive(root, &subdir, projects, standalone, in_project);
    }
}

fn check_dir_for_project(dir: &Path, rel_path: &str, out: &mut Vec<SubProject>) -> bool {
    for &(marker, kind) in PROJECT_MARKERS {
        if dir.join(marker).exists() {
            let is_workspace = check_workspace_marker(&dir.join(marker), kind);
            out.push(SubProject {
                path: rel_path.into(),
                kind: kind.into(),
                is_workspace,
            });
            return true;
        }
    }
    false
}

fn check_workspace_marker(path: &Path, kind: &str) -> bool {
    match kind {
        "rust/cargo" => fs::read_to_string(path)
            .map(|c| c.contains("[workspace]"))
            .unwrap_or(false),
        "node" => fs::read_to_string(path)
            .map(|c| c.contains("\"workspaces\""))
            .unwrap_or(false),
        _ => false,
    }
}

fn is_source_file(name: &str) -> bool {
    !detect_language_id(name).is_empty()
}

fn is_hidden_or_ignored(name: &str) -> bool {
    name.starts_with('.')
        || name == "target"
        || name == "node_modules"
        || name == "vendor"
        || name == "__pycache__"
        || name == "build"
        || name == "dist"
}

fn format_output(subprojects: &[SubProject], standalone: &BTreeMap<String, Vec<String>>) -> String {
    let mut out = String::new();

    if !subprojects.is_empty() {
        writeln!(out, "Subprojects:").ok();
        for sp in subprojects {
            let ws = if sp.is_workspace { " (workspace)" } else { "" };
            writeln!(out, "  {}: {}{}", sp.path, sp.kind, ws).ok();
        }
    }

    if !standalone.is_empty() {
        if !out.is_empty() {
            writeln!(out).ok();
        }
        writeln!(out, "Standalone files:").ok();
        for (dir, files) in standalone {
            if files.len() <= 3 {
                let names = files.join(", ");
                writeln!(out, "  {dir}/: {names}").ok();
            } else {
                writeln!(out, "  {dir}/: {} source files", files.len()).ok();
            }
        }
    }

    if out.is_empty() {
        "No subprojects or standalone files found.".into()
    } else {
        out.trim_end().to_string()
    }
}
