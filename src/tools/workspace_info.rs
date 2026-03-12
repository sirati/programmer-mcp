//! Workspace discovery: sub-projects, workspaces, and standalone files.

use std::collections::BTreeMap;
use std::fmt::Write;
use std::path::Path;

/// Project manifest files that indicate a sub-project.
const PROJECT_MARKERS: &[(&str, &str)] = &[
    ("Cargo.toml", "Rust (Cargo)"),
    ("package.json", "Node.js"),
    ("go.mod", "Go"),
    ("pyproject.toml", "Python"),
    ("setup.py", "Python"),
    ("flake.nix", "Nix flake"),
    ("default.nix", "Nix"),
    ("Makefile", "Make"),
    ("CMakeLists.txt", "CMake"),
    ("build.gradle", "Gradle"),
    ("build.gradle.kts", "Gradle (Kotlin)"),
    ("pom.xml", "Maven"),
    ("Gemfile", "Ruby"),
    ("mix.exs", "Elixir"),
    ("dune-project", "OCaml (Dune)"),
    ("build.zig", "Zig"),
];

/// Source file extensions we consider "standalone" (not in a sub-project).
const SOURCE_EXTS: &[&str] = &[
    "rs", "py", "go", "ts", "tsx", "js", "jsx", "c", "cpp", "h", "hpp", "java", "kt", "rb", "cs",
    "swift", "zig", "lua", "ml", "mli", "nix", "sh",
];

/// Collect workspace information: sub-projects, their types, and standalone files.
pub fn collect_workspace_info() -> String {
    let cwd = match std::env::current_dir() {
        Ok(p) => p,
        Err(e) => return format!("Failed to read workspace: {e}"),
    };

    let mut subprojects: Vec<(String, String)> = Vec::new(); // (path, type)
    let mut dir_files: BTreeMap<String, Vec<String>> = BTreeMap::new(); // dir -> files

    scan_directory(&cwd, &cwd, 0, 3, &mut subprojects, &mut dir_files);

    let mut output = String::new();

    // Root project info
    let root_markers = detect_project_type(&cwd);
    if !root_markers.is_empty() {
        let _ = writeln!(output, "Root: {}", root_markers.join(", "));
    }
    let _ = writeln!(output, "Workspace: {}\n", cwd.display());

    // Sub-projects
    if !subprojects.is_empty() {
        let _ = writeln!(output, "Sub-projects:");
        for (path, kind) in &subprojects {
            let _ = writeln!(output, "  {path}/ ({kind})");
        }
        output.push('\n');
    }

    // Standalone files by directory
    if !dir_files.is_empty() {
        let _ = writeln!(output, "Standalone files:");
        for (dir, files) in &dir_files {
            let dir_display = if dir.is_empty() { "." } else { dir.as_str() };
            if files.len() <= 3 {
                let _ = writeln!(output, "  {dir_display}/: {}", files.join(", "));
            } else {
                let _ = writeln!(output, "  {dir_display}/: {} files", files.len());
            }
        }
    }

    if subprojects.is_empty() && dir_files.is_empty() {
        output.push_str("No sub-projects or standalone source files found.\n");
    }

    output
}

fn detect_project_type(dir: &Path) -> Vec<String> {
    let mut types = Vec::new();
    for &(marker, kind) in PROJECT_MARKERS {
        if dir.join(marker).exists() {
            types.push(kind.to_string());
        }
    }
    types
}

fn scan_directory(
    base: &Path,
    dir: &Path,
    depth: usize,
    max_depth: usize,
    subprojects: &mut Vec<(String, String)>,
    dir_files: &mut BTreeMap<String, Vec<String>>,
) {
    if depth > max_depth {
        return;
    }

    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return,
    };

    let mut subdirs = Vec::new();
    let mut source_files = Vec::new();

    for entry in entries.flatten() {
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().to_string();

        // Skip hidden dirs and common non-source dirs
        if name.starts_with('.')
            || name == "node_modules"
            || name == "target"
            || name == "vendor"
            || name == "__pycache__"
            || name == "dist"
            || name == "build"
        {
            continue;
        }

        if path.is_dir() {
            subdirs.push((path, name));
        } else if path.is_file() {
            if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
                if SOURCE_EXTS.contains(&ext) {
                    source_files.push(name);
                }
            }
        }
    }

    // Check subdirs for sub-projects
    for (subdir_path, subdir_name) in &subdirs {
        // Don't count root directory markers
        if depth == 0 && subdir_name == "src" {
            // src is part of root project, scan it for files
            scan_directory(
                base,
                subdir_path,
                depth + 1,
                max_depth,
                subprojects,
                dir_files,
            );
            continue;
        }

        let project_types = detect_project_type(subdir_path);
        if !project_types.is_empty() {
            let rel = subdir_path.strip_prefix(base).unwrap_or(subdir_path);
            subprojects.push((rel.display().to_string(), project_types.join(", ")));
            // Don't recurse into sub-projects
            continue;
        }

        scan_directory(
            base,
            subdir_path,
            depth + 1,
            max_depth,
            subprojects,
            dir_files,
        );
    }

    // Record standalone source files (files not in a sub-project directory)
    if !source_files.is_empty() && depth > 0 {
        let rel = dir.strip_prefix(base).unwrap_or(dir);
        let key = rel.display().to_string();
        dir_files.entry(key).or_default().extend(source_files);
    }
}
