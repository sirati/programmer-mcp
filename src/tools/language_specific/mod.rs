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
