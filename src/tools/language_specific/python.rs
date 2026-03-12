/// Check if a line is a Python docstring delimiter or decorator (not code).
pub fn is_doc_or_attr(line: &str) -> bool {
    let t = line.trim();
    t.starts_with("\"\"\"") || t.starts_with("'''") || t.starts_with('@')
}

/// Check if a line looks like a Python function/class signature.
pub fn looks_like_signature(line: &str) -> bool {
    let t = line.trim_start();
    t.starts_with("def ") || t.starts_with("async def ") || t.starts_with("class ")
}
