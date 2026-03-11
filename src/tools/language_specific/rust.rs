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
