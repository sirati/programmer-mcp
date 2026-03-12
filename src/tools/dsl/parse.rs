//! Low-level DSL text parsing utilities.
//!
//! Handles comment stripping, word splitting, brace expansion, and item-list parsing.

/// Strip a trailing `#`-style comment from a line.
pub fn strip_comment(line: &str) -> &str {
    // Only strip if '#' is not inside braces or brackets
    let mut depth = 0usize;
    for (i, c) in line.char_indices() {
        match c {
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

/// Split `s` by whitespace/commas while respecting `{...}` nesting depth.
fn split_respecting_braces(s: &str) -> Vec<String> {
    let mut items: Vec<String> = Vec::new();
    let mut current = String::new();
    let mut depth = 0usize;

    for c in s.chars() {
        match c {
            '{' => {
                depth += 1;
                current.push(c);
            }
            '}' => {
                depth = depth.saturating_sub(1);
                current.push(c);
            }
            ',' | ' ' | '\t' if depth == 0 => {
                let trimmed = current.trim().to_string();
                if !trimmed.is_empty() {
                    items.push(trimmed);
                }
                current.clear();
            }
            _ => current.push(c),
        }
    }
    let trimmed = current.trim().to_string();
    if !trimmed.is_empty() {
        items.push(trimmed);
    }
    items
}

/// Parse a DSL item list: `[a, b, tools/{mod.rs x.rs}]` or bare `a b c`.
///
/// Returns the fully expanded list of strings, deduplicated while preserving order.
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
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_strip_comment() {
        assert_eq!(strip_comment("cd src  # navigate"), "cd src  ");
        assert_eq!(
            strip_comment("body [foo{a,b}] # comment"),
            "body [foo{a,b}] "
        );
        assert_eq!(strip_comment("no comment"), "no comment");
    }

    #[test]
    fn test_expand_braces_simple() {
        let mut r = expand_braces("tools/{mod.rs x.rs}");
        r.sort();
        assert_eq!(r, vec!["tools/mod.rs", "tools/x.rs"]);
    }

    #[test]
    fn test_expand_braces_empty() {
        assert_eq!(expand_braces(".{}"), vec!["."]);
    }

    #[test]
    fn test_expand_braces_no_brace() {
        assert_eq!(expand_braces("main.rs"), vec!["main.rs"]);
    }

    #[test]
    fn test_parse_item_list() {
        let items = parse_item_list("[main, tools/{mod.rs x.rs}]");
        assert_eq!(items, vec!["main", "tools/mod.rs", "tools/x.rs"]);
    }

    #[test]
    fn test_parse_item_list_bare() {
        let items = parse_item_list("a b c");
        assert_eq!(items, vec!["a", "b", "c"]);
    }
}
