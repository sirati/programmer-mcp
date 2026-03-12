//! Rust-specific diagnostic message processing.
//!
//! Strips noise added by rustc/rust-analyzer that clutters output,
//! such as lint attribute annotations and default-on notices.

/// Clean a Rust diagnostic message by removing noise lines.
///
/// Removes lines like:
/// - `` `#[warn(unused_imports)]` (part of `#[warn(unused)]`) on by default ``
/// - `` `#[warn(deprecated)]` on by default ``
pub fn clean_message(message: &str) -> String {
    message
        .lines()
        .filter(|line| !is_noise_line(line))
        .collect::<Vec<_>>()
        .join("\n")
        .trim()
        .to_string()
}

fn is_noise_line(line: &str) -> bool {
    let trimmed = line.trim();
    // "`#[warn(unused_imports)]` (part of `#[warn(unused)]`) on by default"
    if trimmed.starts_with("`#[") && trimmed.contains("on by default") {
        return true;
    }
    // Bare lint attribute lines like "`#[warn(deprecated)]`"
    if trimmed.starts_with("`#[") && trimmed.ends_with("]`") {
        return true;
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strips_warn_on_by_default() {
        let msg = "unused import: `futures::StreamExt`\n\
                   `#[warn(unused_imports)]` (part of `#[warn(unused)]`) on by default";
        assert_eq!(clean_message(msg), "unused import: `futures::StreamExt`");
    }

    #[test]
    fn strips_deprecated_on_by_default() {
        let msg = "use of deprecated field `root_uri`\n\
                   `#[warn(deprecated)]` on by default";
        assert_eq!(clean_message(msg), "use of deprecated field `root_uri`");
    }

    #[test]
    fn preserves_clean_message() {
        let msg = "cannot find value `foo` in this scope";
        assert_eq!(clean_message(msg), msg);
    }

    #[test]
    fn strips_bare_lint_attr() {
        let msg = "some warning\n`#[warn(dead_code)]`";
        assert_eq!(clean_message(msg), "some warning");
    }
}
