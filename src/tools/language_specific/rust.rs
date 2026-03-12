/// Rust keywords and primitives whose long-form docs should be collapsed.
pub const KEYWORDS: &[&str] = &[
    "struct", "enum", "trait", "impl", "fn", "mod", "use", "pub", "let", "mut", "const", "static",
    "type", "where", "match", "if", "else", "for", "while", "loop", "return", "async", "await",
    "move", "ref", "self", "super", "crate", "dyn", "unsafe", "bool", "char", "str", "i8", "i16",
    "i32", "i64", "i128", "isize", "u8", "u16", "u32", "u64", "u128", "usize", "f32", "f64",
];

/// Check if a line is rust-analyzer noise (size/align metadata).
pub fn is_noise_line(line: &str) -> bool {
    let trimmed = line.trim();
    trimmed.starts_with("size = ") || trimmed.starts_with("align = ")
}

/// Check if a line is a Rust doc comment or attribute (not code).
pub fn is_doc_or_attr(line: &str) -> bool {
    let t = line.trim();
    t.starts_with("///")
        || t.starts_with("//!")
        || t.starts_with("/**")
        || t.starts_with("* ")
        || t.starts_with("*/")
        || t == "*"
        || (t.starts_with('#') && t.contains('['))
}

/// Check if a line looks like a Rust function/type signature.
pub fn looks_like_signature(line: &str) -> bool {
    let t = line.trim_start();
    t.starts_with("fn ")
        || t.starts_with("pub fn ")
        || t.starts_with("pub(crate) fn ")
        || t.starts_with("pub(super) fn ")
        || t.starts_with("async fn ")
        || t.starts_with("pub async fn ")
        || t.starts_with("unsafe fn ")
        || t.starts_with("const fn ")
        || t.starts_with("pub const fn ")
        || t.starts_with("struct ")
        || t.starts_with("pub struct ")
        || t.starts_with("enum ")
        || t.starts_with("pub enum ")
        || t.starts_with("impl ")
        || t.starts_with("impl<")
        || t.starts_with("trait ")
        || t.starts_with("pub trait ")
}
