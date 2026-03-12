pub mod go;
pub mod javascript;
pub mod python;
pub mod rust;

/// Check if hover text looks like a language keyword/primitive doc that should be collapsed.
/// Returns the keyword name if so, None otherwise.
pub fn detect_keyword_doc<'a>(language: &str, text: &'a str) -> Option<&'a str> {
    let first = text.lines().find(|l| !l.trim().is_empty())?.trim();
    let keywords = match language {
        "rust" => &rust::KEYWORDS[..],
        _ => return None,
    };
    if keywords.contains(&first) {
        Some(first)
    } else {
        None
    }
}

/// Check if a hover line is language-specific noise that should be stripped.
pub fn is_noise_line(language: &str, line: &str) -> bool {
    match language {
        "rust" => rust::is_noise_line(line),
        _ => false,
    }
}

/// Detect language from a file path extension.
pub fn lang_from_path(path: &str) -> Option<&'static str> {
    let ext = path.rsplit('.').next()?;
    match ext {
        "rs" => Some("rust"),
        "go" => Some("go"),
        "py" | "pyi" => Some("python"),
        "js" | "jsx" | "ts" | "tsx" | "mjs" | "cjs" => Some("javascript"),
        _ => None,
    }
}

/// Check if a line is a doc comment or attribute for the given language.
/// Falls back to checking all languages if `lang` is None.
pub fn is_doc_or_attr(lang: Option<&str>, line: &str) -> bool {
    match lang {
        Some("rust") => rust::is_doc_or_attr(line),
        Some("go") => go::is_doc_or_attr(line),
        Some("python") => python::is_doc_or_attr(line),
        Some("javascript") => javascript::is_doc_or_attr(line),
        _ => {
            // Unknown language — check all
            rust::is_doc_or_attr(line)
                || go::is_doc_or_attr(line)
                || python::is_doc_or_attr(line)
                || javascript::is_doc_or_attr(line)
        }
    }
}

/// Check if a line looks like a function/type signature for the given language.
/// Falls back to checking all languages if `lang` is None.
pub fn looks_like_signature(lang: Option<&str>, line: &str) -> bool {
    match lang {
        Some("rust") => rust::looks_like_signature(line),
        Some("go") => go::looks_like_signature(line),
        Some("python") => python::looks_like_signature(line),
        Some("javascript") => javascript::looks_like_signature(line),
        _ => {
            // Unknown language — check all
            rust::looks_like_signature(line)
                || go::looks_like_signature(line)
                || python::looks_like_signature(line)
                || javascript::looks_like_signature(line)
        }
    }
}
