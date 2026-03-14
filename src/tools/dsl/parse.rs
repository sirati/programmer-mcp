//! Low-level DSL text parsing utilities.
//!
//! Handles comment stripping, word splitting, brace expansion, and item-list parsing.

/// Strip a trailing `#`-style comment from a line.
/// Respects braces, brackets, and quotes.
pub fn strip_comment(line: &str) -> &str {
    let mut depth = 0usize;
    let mut in_quote: Option<char> = None;
    let mut escape = false;
    for (i, c) in line.char_indices() {
        if escape {
            escape = false;
            continue;
        }
        if c == '\\' {
            escape = true;
            continue;
        }
        if let Some(q) = in_quote {
            if c == q {
                in_quote = None;
            }
            continue;
        }
        match c {
            '"' | '\'' => in_quote = Some(c),
            '[' | '{' => depth += 1,
            ']' | '}' => depth = depth.saturating_sub(1),
            '#' if depth == 0 => return &line[..i],
            _ => {}
        }
    }
    line
}

/// Split a line into (first_word, remainder).
pub fn split_first_word(s: &str) -> (&str, &str) {
    let s = s.trim();
    match s.find(|c: char| c.is_whitespace()) {
        Some(i) => (&s[..i], s[i..].trim_start()),
        None => (s, ""),
    }
}

/// Remove surrounding quotes from a string and unescape `\"`, `\'`, `\\`.
/// If the string is not quoted, returns it unchanged.
pub fn unquote(s: &str) -> String {
    let s = s.trim();
    let (quote, inner) =
        if (s.starts_with('"') && s.ends_with('"')) || (s.starts_with('\'') && s.ends_with('\'')) {
            (s.chars().next(), &s[1..s.len() - 1])
        } else {
            return s.to_string();
        };

    let q = quote.unwrap();
    let mut result = String::with_capacity(inner.len());
    let mut escape = false;
    for c in inner.chars() {
        if escape {
            // Only unescape the quote char and backslash itself
            if c == q || c == '\\' {
                result.push(c);
            } else {
                result.push('\\');
                result.push(c);
            }
            escape = false;
        } else if c == '\\' {
            escape = true;
        } else {
            result.push(c);
        }
    }
    if escape {
        result.push('\\');
    }
    result
}

/// Split a string into words, respecting quotes and braces.
/// Quoted strings are kept as a single token (with quotes retained for later unquoting).
pub fn split_words(s: &str) -> Vec<String> {
    let mut words = Vec::new();
    let mut current = String::new();
    let mut in_quote: Option<char> = None;
    let mut depth = 0usize;
    let mut escape = false;

    for c in s.chars() {
        if escape {
            current.push(c);
            escape = false;
            continue;
        }
        if c == '\\' && in_quote.is_some() {
            current.push(c);
            escape = true;
            continue;
        }
        if let Some(q) = in_quote {
            current.push(c);
            if c == q {
                in_quote = None;
            }
            continue;
        }
        match c {
            '"' | '\'' => {
                current.push(c);
                in_quote = Some(c);
            }
            '{' => {
                depth += 1;
                current.push(c);
            }
            '}' => {
                depth = depth.saturating_sub(1);
                current.push(c);
            }
            ' ' | '\t' | ',' if depth == 0 => {
                let trimmed = current.trim().to_string();
                if !trimmed.is_empty() {
                    words.push(trimmed);
                }
                current.clear();
            }
            _ => current.push(c),
        }
    }
    let trimmed = current.trim().to_string();
    if !trimmed.is_empty() {
        words.push(trimmed);
    }
    words
}

/// Split a line by `|` pipe separator, respecting quotes.
pub fn split_pipe(line: &str) -> Vec<String> {
    let mut segments = Vec::new();
    let mut current = String::new();
    let mut in_quote: Option<char> = None;
    let mut escape = false;

    for c in line.chars() {
        if escape {
            current.push(c);
            escape = false;
            continue;
        }
        if c == '\\' && in_quote.is_some() {
            current.push(c);
            escape = true;
            continue;
        }
        if let Some(q) = in_quote {
            current.push(c);
            if c == q {
                in_quote = None;
            }
            continue;
        }
        match c {
            '"' | '\'' => {
                current.push(c);
                in_quote = Some(c);
            }
            '|' => {
                segments.push(std::mem::take(&mut current));
            }
            _ => current.push(c),
        }
    }
    segments.push(current);
    segments
}

/// Find the position of the closing `}` that matches the opening `{` at position 0 of `s`.
/// `s` must start with `{`.
fn find_matching_close(s: &str) -> Option<usize> {
    let mut depth = 0usize;
    for (i, c) in s.char_indices() {
        match c {
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    return Some(i);
                }
            }
            _ => {}
        }
    }
    None
}

/// Expand bash-style brace expressions in a single token.
///
/// `tools/{mod.rs symbol_info.rs}` → `["tools/mod.rs", "tools/symbol_info.rs"]`
/// `find_{a ,b}` → `["find_a", "find_b"]`
/// `.{}` → `["."]` (empty braces = empty string alternative)
pub fn expand_braces(s: &str) -> Vec<String> {
    let Some(open) = s.find('{') else {
        return vec![s.to_string()];
    };

    let prefix = &s[..open];
    let after_open = &s[open..]; // includes '{'

    let Some(close_rel) = find_matching_close(after_open) else {
        // No matching '}' – return as-is
        return vec![s.to_string()];
    };

    let inner = &after_open[1..close_rel]; // content between { }
    let suffix = &after_open[close_rel + 1..];

    // Split inner content by commas and whitespace
    let alternatives: Vec<&str> = inner
        .split(|c: char| c == ',' || c.is_whitespace())
        .map(str::trim)
        .collect();

    if alternatives.iter().all(|a| a.is_empty()) {
        // {} or {,} etc → single empty alternative = prefix + suffix
        return expand_braces(&format!("{prefix}{suffix}"));
    }

    alternatives
        .iter()
        .flat_map(|alt| {
            let combined = format!("{prefix}{alt}{suffix}");
            expand_braces(&combined)
        })
        .collect()
}

/// Split `s` by whitespace/commas while respecting `{...}` and quote nesting.
fn split_respecting_braces(s: &str) -> Vec<String> {
    split_words(s)
}

/// Parse a DSL item list: `[a, b, tools/{mod.rs x.rs}]` or bare `a b c`.
///
/// Returns the fully expanded list of strings, deduplicated while preserving order.
/// Quoted items are unquoted after expansion.
pub fn parse_item_list(s: &str) -> Vec<String> {
    let s = s.trim();
    let inner = if s.starts_with('[') && s.ends_with(']') {
        &s[1..s.len() - 1]
    } else {
        s
    };

    split_respecting_braces(inner)
        .into_iter()
        .flat_map(|item| expand_braces(&item))
        .filter(|s| !s.is_empty())
        .map(|s| unquote(&s))
        .collect()
}

#[cfg(test)]
#[path = "parse_tests.rs"]
mod tests;
