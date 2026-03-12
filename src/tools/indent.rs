#![allow(dead_code)]
/// Indentation detection and normalization for code editing.
///
/// Detects the indent style (tabs vs spaces, width) of existing code and
/// normalizes replacement content to match.

/// Detected indentation style.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IndentStyle {
    Tabs,
    Spaces(u32),
}

impl IndentStyle {
    /// Produce a single indent level in this style.
    pub fn unit(&self) -> &str {
        match self {
            IndentStyle::Tabs => "\t",
            IndentStyle::Spaces(2) => "  ",
            IndentStyle::Spaces(4) => "    ",
            IndentStyle::Spaces(8) => "        ",
            _ => "    ", // fallback
        }
    }
}

/// Detect the indent style used in a block of text.
///
/// Looks at lines that start with whitespace and votes on tabs vs spaces.
/// For spaces, detects the most common width (2 or 4).
pub fn detect_indent_style(text: &str) -> IndentStyle {
    let mut tab_votes = 0u32;
    let mut space_votes = 0u32;
    let mut widths: Vec<u32> = Vec::new();

    for line in text.lines() {
        if line.is_empty() {
            continue;
        }
        let leading = line.len() - line.trim_start().len();
        if leading == 0 {
            continue;
        }
        if line.starts_with('\t') {
            tab_votes += 1;
        } else if line.starts_with(' ') {
            space_votes += 1;
            widths.push(leading as u32);
        }
    }

    if tab_votes > space_votes {
        return IndentStyle::Tabs;
    }

    if widths.is_empty() {
        return IndentStyle::Spaces(4); // default
    }

    // Find the GCD of all indent widths to determine the base unit
    let gcd = widths.iter().copied().reduce(gcd_u32).unwrap_or(4);
    if gcd == 0 {
        IndentStyle::Spaces(4)
    } else {
        IndentStyle::Spaces(gcd.min(8))
    }
}

fn gcd_u32(a: u32, b: u32) -> u32 {
    if b == 0 {
        a
    } else {
        gcd_u32(b, a % b)
    }
}

/// Count the base indentation level (in characters) of the first non-empty line.
pub fn base_indent_chars(text: &str) -> usize {
    for line in text.lines() {
        if !line.trim().is_empty() {
            return line.len() - line.trim_start().len();
        }
    }
    0
}

/// Detect line ending style. Returns "\r\n" if CRLF is dominant, else "\n".
pub fn detect_line_ending(text: &str) -> &'static str {
    let crlf = text.matches("\r\n").count();
    let lf_only = text.matches('\n').count().saturating_sub(crlf);
    if crlf > lf_only {
        "\r\n"
    } else {
        "\n"
    }
}

/// Normalize `new_content` to match the indentation of existing code at `target_indent`.
///
/// 1. Strips the base indent from `new_content` (the indent of its first non-empty line)
/// 2. Re-indents every line to `target_indent` + relative indent
/// 3. Normalizes line endings to match `target_line_ending`
pub fn normalize_indent(
    new_content: &str,
    target_indent: &str,
    target_line_ending: &str,
) -> String {
    let new_base = base_indent_chars(new_content);
    let lines: Vec<&str> = new_content.lines().collect();
    let mut result = Vec::with_capacity(lines.len());

    for line in &lines {
        if line.trim().is_empty() {
            result.push(String::new());
            continue;
        }
        let current_indent = line.len() - line.trim_start().len();
        // Relative indent beyond the base
        let relative = current_indent.saturating_sub(new_base);
        let relative_ws: String = if target_indent.contains('\t') {
            // Tab mode: convert relative chars to tabs (assuming 1 tab per base unit)
            "\t".repeat(relative / 4_usize.max(1))
        } else {
            " ".repeat(relative)
        };
        result.push(format!("{target_indent}{relative_ws}{}", line.trim_start()));
    }

    result.join(target_line_ending)
}

/// Extract the leading whitespace from a string.
pub fn leading_whitespace(line: &str) -> &str {
    let trimmed = line.trim_start();
    &line[..line.len() - trimmed.len()]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detect_spaces_4() {
        let code = "fn foo() {\n    let x = 1;\n    if true {\n        bar();\n    }\n}";
        assert_eq!(detect_indent_style(code), IndentStyle::Spaces(4));
    }

    #[test]
    fn detect_spaces_2() {
        let code = "fn foo() {\n  let x = 1;\n  if true {\n    bar();\n  }\n}";
        assert_eq!(detect_indent_style(code), IndentStyle::Spaces(2));
    }

    #[test]
    fn detect_tabs() {
        let code = "fn foo() {\n\tlet x = 1;\n\tif true {\n\t\tbar();\n\t}\n}";
        assert_eq!(detect_indent_style(code), IndentStyle::Tabs);
    }

    #[test]
    fn normalize_strips_base_and_reindents() {
        let new_content = "    fn bar() {\n        baz();\n    }";
        let result = normalize_indent(new_content, "        ", "\n");
        assert_eq!(result, "        fn bar() {\n            baz();\n        }");
    }

    #[test]
    fn normalize_handles_no_indent() {
        let new_content = "fn bar() {\n    baz();\n}";
        let result = normalize_indent(new_content, "    ", "\n");
        assert_eq!(result, "    fn bar() {\n        baz();\n    }");
    }

    #[test]
    fn line_ending_detection() {
        assert_eq!(detect_line_ending("a\r\nb\r\nc"), "\r\n");
        assert_eq!(detect_line_ending("a\nb\nc"), "\n");
    }
}
