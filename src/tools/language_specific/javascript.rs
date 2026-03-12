/// Check if a line is a JS/TS doc comment or decorator (not code).
pub fn is_doc_or_attr(line: &str) -> bool {
    let t = line.trim();
    t.starts_with("/**")
        || t.starts_with("* ")
        || t.starts_with("*/")
        || t == "*"
        || t.starts_with("//")
        || t.starts_with('@')
}

/// Check if a line looks like a JS/TS function/class signature.
pub fn looks_like_signature(line: &str) -> bool {
    let t = line.trim_start();
    t.starts_with("function ")
        || t.starts_with("export function ")
        || t.starts_with("export default function ")
        || t.starts_with("export class ")
        || t.starts_with("class ")
        || t.starts_with("const ")
        || t.starts_with("export const ")
}
