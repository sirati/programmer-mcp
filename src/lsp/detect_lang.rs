use std::path::Path;

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

/// Map a user-provided language name to possible file extensions.
/// Used when searching for files by language.
pub fn language_extensions(language: &str) -> &'static [&'static str] {
    match language.to_lowercase().as_str() {
        "rust" => &["rs"],
        "go" => &["go"],
        "python" => &["py", "pyi"],
        "javascript" => &["js", "mjs", "cjs"],
        "typescript" => &["ts", "mts", "cts"],
        "c" => &["c", "h"],
        "cpp" | "c++" => &["cpp", "cxx", "cc", "hpp", "h"],
        "java" => &["java"],
        "ruby" => &["rb"],
        "nix" => &["nix"],
        "zig" => &["zig"],
        _ => &[],
    }
}
