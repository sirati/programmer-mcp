use std::fmt::Write;
use std::sync::Arc;

use lsp_types::{DiagnosticSeverity, Location};

use super::formatting::{format_lines_with_gaps, lines_to_display, path_to_uri};
use crate::lsp::client::{LspClient, LspClientError};

/// Get diagnostics for a file, with optional context lines and line numbers.
pub async fn get_diagnostics(
    client: &Arc<LspClient>,
    file_path: &str,
    context_lines: usize,
    show_line_numbers: bool,
) -> Result<String, LspClientError> {
    client.open_file(file_path).await?;

    // Wait briefly for diagnostics to arrive
    tokio::time::sleep(std::time::Duration::from_secs(2)).await;

    let uri = path_to_uri(file_path).map_err(LspClientError::Other)?;

    // Trigger pull diagnostics if supported
    let _ = client.diagnostic(&uri).await;

    let diagnostics = client.get_cached_diagnostics(&uri).await;

    if diagnostics.is_empty() {
        return Ok(format!("No diagnostics found for {file_path}"));
    }

    let mut output = format!("{file_path}\nDiagnostics in File: {}\n", diagnostics.len());

    // Summary of each diagnostic
    for diag in &diagnostics {
        let severity = match diag.severity {
            Some(DiagnosticSeverity::ERROR) => "Error",
            Some(DiagnosticSeverity::WARNING) => "Warning",
            Some(DiagnosticSeverity::INFORMATION) => "Info",
            Some(DiagnosticSeverity::HINT) => "Hint",
            _ => "Unknown",
        };

        let location = format!(
            "L{}:C{}",
            diag.range.start.line + 1,
            diag.range.start.character + 1,
        );

        let _ = write!(output, "{severity} at {location}: {}", diag.message);

        if let Some(source) = &diag.source {
            let _ = write!(output, " (Source: {source}");
            if let Some(code) = &diag.code {
                let _ = write!(output, ", Code: {code:?}");
            }
            output.push(')');
        }
        output.push('\n');
    }

    // Show file content with context if requested
    if show_line_numbers {
        let content = std::fs::read_to_string(file_path)
            .map_err(|e| LspClientError::Other(format!("read error: {e}")))?;
        let lines: Vec<&str> = content.lines().collect();

        let locations: Vec<Location> = diagnostics
            .iter()
            .map(|d| Location {
                uri: uri.clone(),
                range: d.range,
            })
            .collect();

        let visible = lines_to_display(&locations, lines.len(), context_lines);
        let formatted = format_lines_with_gaps(&lines, &visible);
        let _ = write!(output, "\n{formatted}");
    }

    Ok(output)
}
