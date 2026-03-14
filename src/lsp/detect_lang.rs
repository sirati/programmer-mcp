use std::path::Path;

/// All source file extensions the tool considers relevant.
///
/// This is the single source of truth — kept in sync with [`detect_language_id`].
pub const SOURCE_EXTS: &[&str] = &[
    "rs", "go", "py", "pyi", "js", "jsx", "ts", "tsx", "c", "cc", "cpp", "cxx", "h", "hpp", "cs",
    "java", "rb", "php", "swift", "kt", "kts", "scala", "hs", "ex", "exs", "erl", "hrl", "clj",
    "lua", "r", "dart", "zig", "nim", "ml", "mli", "nix", "sh", "bash", "zsh", "css", "scss",
    "html", "htm", "json", "yaml", "yml", "toml", "xml", "sql", "md", "markdown", "tex", "latex",
    "fs", "el",
];

/// Detect LSP language ID from a file path's extension.
pub fn detect_language_id(path: &str) -> &'static str {
    let ext = Path::new(path)
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("");

    match ext.to_lowercase().as_str() {
        "rs" => "rust",
        "go" => "go",
        "py" | "pyi" => "python",
        "js" => "javascript",
        "jsx" => "javascriptreact",
        "ts" => "typescript",
        "tsx" => "typescriptreact",
        "c" => "c",
        "cpp" | "cxx" | "cc" | "c++" => "cpp",
        "h" | "hpp" => "cpp",
        "cs" => "csharp",
        "java" => "java",
        "rb" => "ruby",
        "php" => "php",
        "swift" => "swift",
        "kt" | "kts" => "kotlin",
        "scala" => "scala",
        "hs" => "haskell",
        "ex" | "exs" => "elixir",
        "erl" | "hrl" => "erlang",
        "clj" => "clojure",
        "lua" => "lua",
        "r" => "r",
        "dart" => "dart",
        "zig" => "zig",
        "nim" => "nim",
        "ml" | "mli" => "ocaml",
        "nix" => "nix",
        "sh" | "bash" | "zsh" => "shellscript",
        "css" => "css",
        "scss" => "scss",
        "html" | "htm" => "html",
        "json" => "json",
        "yaml" | "yml" => "yaml",
        "toml" => "toml",
        "xml" => "xml",
        "sql" => "sql",
        "md" | "markdown" => "markdown",
        "tex" | "latex" => "latex",
        _ => "",
    }
}

/// Detect the dominant programming language in a directory.
/// Checks project marker files first (go.mod, Cargo.toml, etc.),
/// then falls back to sampling source file extensions.
pub fn detect_dir_language(dir: &Path) -> Option<String> {
    if dir.as_os_str().is_empty() {
        return None;
    }

    let markers: &[(&str, &str)] = &[
        ("go.mod", "go"),
        ("Cargo.toml", "rust"),
        ("package.json", "typescript"),
        ("pyproject.toml", "python"),
        ("setup.py", "python"),
        ("flake.nix", "nix"),
        ("default.nix", "nix"),
        ("Makefile", "make"),
    ];
    for (marker, lang) in markers {
        if dir.join(marker).exists() {
            return Some(lang.to_string());
        }
    }

    let entries = std::fs::read_dir(dir).ok()?;
    let mut counts: std::collections::HashMap<String, usize> = std::collections::HashMap::new();

    for entry in entries.take(50).flatten() {
        let path = entry.path();
        if path.is_file() {
            let lang = detect_language_id(&path.display().to_string());
            if !lang.is_empty() {
                *counts.entry(lang.to_string()).or_default() += 1;
            }
        }
    }

    counts
        .into_iter()
        .max_by_key(|(_, count)| *count)
        .map(|(lang, _)| lang)
}
