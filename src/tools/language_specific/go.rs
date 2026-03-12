/// Check if a line is a Go doc comment (not code).
pub fn is_doc_or_attr(line: &str) -> bool {
    let t = line.trim();
    t.starts_with("//")
}

/// Check if a line looks like a Go function/type signature.
pub fn looks_like_signature(line: &str) -> bool {
    let t = line.trim_start();
    t.starts_with("func ")
        || t.starts_with("type ")
        || t.starts_with("var ")
        || t.starts_with("const ")
}
