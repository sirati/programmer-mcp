use std::collections::HashSet;
use std::fmt::Write;
use std::sync::Arc;

use tracing::debug;

use super::formatting::{
    find_containing_symbol_range, read_range_from_file, relative_to, uri_to_path,
};
use super::symbol_info::not_found_msg;
use super::symbol_search::find_symbol_with_fallback;
use crate::lsp::client::{LspClient, LspClientError};

/// Read the definition location + signature of a symbol.
///
/// Shows: relative file path, kind, container, line range, docstring, and
/// the first few lines of the body (the signature). Use `body` instead
/// to get the full source code.
pub async fn read_definition(
    client: &Arc<LspClient>,
    symbol_name: &str,
    search_dir: Option<&str>,
) -> Result<String, LspClientError> {
    let symbols = find_symbol_with_fallback(client, symbol_name, search_dir).await?;

    if symbols.is_empty() {
        return Ok(not_found_msg(client, symbol_name).await);
    }

    const MAX_RESULTS: usize = 10;
    let total = symbols.len();
    let truncated = total > MAX_RESULTS;

    let ws_root = client.workspace_root();
    let mut output = String::new();
    let mut seen = HashSet::new();
    let mut count = 0;

    for symbol in &symbols {
        if count >= MAX_RESULTS {
            break;
        }
        // Deduplicate by URI + start position
        let key = (
            symbol.location.uri.as_str().to_string(),
            symbol.location.range.start.line,
            symbol.location.range.start.character,
        );
        if !seen.insert(key) {
            continue;
        }
        let loc = &symbol.location;
        let path = uri_to_path(&loc.uri).unwrap_or_else(|| loc.uri.as_str().to_string());
        let rel_path = relative_to(&path, ws_root);

        // Open the file so the LSP tracks it
        if let Err(e) = client.open_file(&path).await {
            debug!("error opening file {path}: {e}");
            continue;
        }

        // Try to get full definition range via document symbols
        let full_range = find_containing_symbol_range(client, &loc.uri, loc.range.start).await;
        let range = full_range.unwrap_or(loc.range);

        let kind = format!("{:?}", symbol.kind);
        let container = symbol
            .container_name
            .as_ref()
            .map(|c| format!("Container: {c}\n"))
            .unwrap_or_default();

        // Extract docstring (lines above the definition)
        let start_line = range.start.line as usize;
        let docstring = extract_doc_lines(&path, start_line);

        // Extract signature: first few lines of the body, up to the opening brace/body
        let signature = match read_range_from_file(&loc.uri, &range) {
            Ok(text) => extract_signature(&text),
            Err(e) => {
                debug!("error reading range: {e}");
                continue;
            }
        };

        let _ = write!(
            output,
            "---\n\n\
             Symbol: {}\n\
             File: {rel_path}\n\
             Kind: {kind}\n\
             {container}\
             Range: L{}:C{} - L{}:C{}\n",
            symbol.name,
            range.start.line + 1,
            range.start.character + 1,
            range.end.line + 1,
            range.end.character + 1,
        );

        if !docstring.is_empty() {
            let _ = write!(output, "\n{docstring}\n");
        }

        let _ = write!(output, "\n{signature}\n");
        count += 1;
    }

    if output.is_empty() {
        return Ok(not_found_msg(client, symbol_name).await);
    }

    if truncated {
        let _ = write!(
            output,
            "\n... and {} more results (use a more specific query)\n",
            total - MAX_RESULTS
        );
    }

    Ok(output)
}

/// Extract doc comment lines above `start_line` (0-indexed).
fn extract_doc_lines(path: &str, start_line: usize) -> String {
    let Ok(content) = std::fs::read_to_string(path) else {
        return String::new();
    };
    let lines: Vec<&str> = content.lines().collect();
    if start_line == 0 || start_line > lines.len() {
        return String::new();
    }

    // In Go/C/C++/JS/etc, `//` comments above a definition ARE doc comments.
    // In Rust, only `///` is a doc comment; plain `//` is a regular comment.
    let slash_slash_is_doc = !path.ends_with(".rs");

    let mut doc_lines = Vec::new();
    let mut i = start_line.saturating_sub(1);
    loop {
        let trimmed = lines.get(i).map(|l| l.trim()).unwrap_or("");
        if trimmed.starts_with("///")          // Rust doc comments
            || trimmed.starts_with("//!")       // Rust inner doc comments
            || trimmed.starts_with("#[doc")     // Rust doc attributes
            || trimmed.starts_with("#[derive")  // Rust derive attributes
            || trimmed.starts_with("#[cfg")     // Rust cfg attributes
            || (trimmed.starts_with('@') && !trimmed.starts_with("@@"))  // Python/Java decorators
            || trimmed.starts_with("/**")       // JSDoc/Javadoc block start
            || trimmed.starts_with("* ")        // JSDoc/Javadoc continuation
            || trimmed.starts_with("*/")        // JSDoc/Javadoc block end
            || trimmed == "*"                   // Bare JSDoc continuation
            || trimmed.starts_with("\"\"\"")    // Python docstring markers
            || (slash_slash_is_doc && trimmed.starts_with("//"))
        // Go/C/JS doc comments
        {
            doc_lines.push(trimmed);
        } else if trimmed.is_empty() && !doc_lines.is_empty() {
            // Allow blank lines within doc blocks
        } else {
            break;
        }
        if i == 0 {
            break;
        }
        i -= 1;
    }

    if doc_lines.is_empty() {
        return String::new();
    }

    doc_lines.reverse();
    doc_lines.join("\n")
}

/// Extract the signature from a full body text.
/// Skips leading attribute macros (#[...], @decorators) to show the actual
/// function/struct declaration. Returns lines up to the opening `{`.
fn extract_signature(body: &str) -> String {
    let lines: Vec<&str> = body.lines().collect();

    // Skip leading attributes/decorators
    let mut start = 0;
    let mut in_multiline_attr = false;
    for (i, line) in lines.iter().enumerate() {
        let trimmed = line.trim();
        if in_multiline_attr {
            // Inside a multi-line attribute like #[tool(description = "...\n...")]
            if trimmed.contains(")]") {
                in_multiline_attr = false;
                start = i + 1;
            }
            continue;
        }
        if trimmed.starts_with("#[") && !trimmed.starts_with("#[doc") {
            if !trimmed.contains(']') {
                in_multiline_attr = true;
            }
            start = i + 1;
            continue;
        }
        if trimmed.starts_with('@') && !trimmed.starts_with("@@") {
            start = i + 1;
            continue;
        }
        break;
    }

    let sig_lines = &lines[start..];
    let max_lines = 5.min(sig_lines.len());

    for (i, line) in sig_lines.iter().enumerate().take(max_lines) {
        let trimmed = line.trim();
        if trimmed.ends_with('{') || trimmed == "{" {
            return sig_lines[..=i].join("\n");
        }
    }

    // For short definitions (e.g. type aliases, constants), return everything
    if sig_lines.len() <= 5 {
        return sig_lines.join("\n");
    }

    // Otherwise just return the first 5 lines with "..."
    let mut result = sig_lines[..max_lines].join("\n");
    result.push_str("\n  ...");
    result
}
