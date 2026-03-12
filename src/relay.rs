//! Shared MCP JSON-RPC relay channel.
//!
//! Used by both the debug proxy (over child process stdin/stdout)
//! and the remote client (over Unix sockets via SSH).

use std::time::Duration;

use serde_json::Value;
use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncWrite, AsyncWriteExt, BufReader};

const PROTOCOL_VERSION: &str = "2025-06-18";
const INIT_TIMEOUT_SECS: u64 = 10;
const RELAY_TIMEOUT_SECS: u64 = 30;

/// A bidirectional MCP JSON-RPC relay over newline-delimited JSON.
/// Generic over any AsyncRead + AsyncWrite pair.
pub struct RelayChannel<W: AsyncWrite + Unpin, R: AsyncRead + Unpin> {
    writer: W,
    reader: BufReader<R>,
    initialized: bool,
}

impl<W: AsyncWrite + Unpin, R: AsyncRead + Unpin> RelayChannel<W, R> {
    pub fn new(writer: W, reader: R) -> Self {
        Self {
            writer,
            reader: BufReader::new(reader),
            initialized: false,
        }
    }

    /// Send a JSON-RPC request and wait for the matching response.
    /// Automatically initializes the MCP session on first use.
    pub async fn relay(&mut self, request_json: &str) -> anyhow::Result<String> {
        if !self.initialized {
            self.ensure_initialized().await?;
        }
        write_line(&mut self.writer, request_json).await?;
        let expected_id = extract_id(request_json);
        tokio::time::timeout(
            Duration::from_secs(RELAY_TIMEOUT_SECS),
            read_matching_response(&mut self.reader, expected_id),
        )
        .await
        .map_err(|_| anyhow::anyhow!("relay timed out waiting for response"))?
    }

    /// Perform MCP initialize handshake.
    pub async fn ensure_initialized(&mut self) -> anyhow::Result<()> {
        let init_req = serde_json::json!({
            "jsonrpc": "2.0",
            "id": 0,
            "method": "initialize",
            "params": {
                "protocolVersion": PROTOCOL_VERSION,
                "capabilities": {},
                "clientInfo": {
                    "name": "programmer-mcp-relay",
                    "version": env!("CARGO_PKG_VERSION")
                }
            }
        })
        .to_string();

        write_line(&mut self.writer, &init_req).await?;

        tokio::time::timeout(
            Duration::from_secs(INIT_TIMEOUT_SECS),
            read_matching_response(&mut self.reader, Some(serde_json::json!(0))),
        )
        .await
        .map_err(|_| anyhow::anyhow!("MCP initialize timed out after {INIT_TIMEOUT_SECS}s"))?
        .map_err(|e| anyhow::anyhow!("MCP initialize handshake failed: {e}"))?;

        let notif = serde_json::json!({
            "jsonrpc": "2.0",
            "method": "notifications/initialized",
            "params": {}
        })
        .to_string();
        write_line(&mut self.writer, &notif).await?;

        self.initialized = true;
        Ok(())
    }

    pub fn is_initialized(&self) -> bool {
        self.initialized
    }
}

async fn write_line(writer: &mut (impl AsyncWrite + Unpin), line: &str) -> anyhow::Result<()> {
    writer.write_all(line.as_bytes()).await?;
    writer.write_all(b"\n").await?;
    writer.flush().await?;
    Ok(())
}

fn read_matching_response<'a, R: AsyncRead + Unpin>(
    reader: &'a mut BufReader<R>,
    expected_id: Option<Value>,
) -> impl std::future::Future<Output = anyhow::Result<String>> + 'a {
    async move {
        let mut line = String::new();
        loop {
            line.clear();
            let n = reader.read_line(&mut line).await?;
            if n == 0 {
                anyhow::bail!("relay channel closed unexpectedly");
            }
            let trimmed = line.trim();
            if trimmed.is_empty() {
                continue;
            }
            if let Ok(val) = serde_json::from_str::<Value>(trimmed) {
                let response_id = val.get("id").cloned();
                if response_id == expected_id {
                    return Ok(trimmed.to_string());
                }
                tracing::debug!("discarding unmatched relay line");
            }
        }
    }
}

pub fn extract_id(json: &str) -> Option<Value> {
    serde_json::from_str::<Value>(json).ok()?.get("id").cloned()
}

pub fn build_jsonrpc_request(id: u64, method: &str, params: serde_json::Value) -> String {
    serde_json::json!({
        "jsonrpc": "2.0",
        "id": id,
        "method": method,
        "params": params,
    })
    .to_string()
}
