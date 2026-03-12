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
        tracing::debug!(
            relay.initialized = self.initialized,
            "relay: entering relay()"
        );
        if !self.initialized {
            tracing::debug!("relay: calling ensure_initialized");
            self.ensure_initialized().await?;
            tracing::debug!("relay: ensure_initialized complete");
        }
        tracing::debug!("relay: writing request");
        write_line(&mut self.writer, request_json).await?;
        let expected_id = extract_id(request_json);
        tracing::debug!(?expected_id, "relay: waiting for response");
        let result = tokio::time::timeout(
            Duration::from_secs(RELAY_TIMEOUT_SECS),
            read_matching_response(&mut self.reader, expected_id),
        )
        .await
        .map_err(|_| anyhow::anyhow!("relay timed out waiting for response"))?;
        tracing::debug!("relay: got response");
        result
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

        tracing::debug!("relay: sending initialize to downstream server");
        write_line(&mut self.writer, &init_req).await?;
        tracing::debug!("relay: waiting for initialize response (timeout={INIT_TIMEOUT_SECS}s)");

        tokio::time::timeout(
            Duration::from_secs(INIT_TIMEOUT_SECS),
            read_matching_response(&mut self.reader, Some(serde_json::json!(0))),
        )
        .await
        .map_err(|_| anyhow::anyhow!("MCP initialize timed out after {INIT_TIMEOUT_SECS}s"))?
        .map_err(|e| anyhow::anyhow!("MCP initialize handshake failed: {e}"))?;

        tracing::debug!("relay: received initialize response, sending notifications/initialized");
        let notif = serde_json::json!({
            "jsonrpc": "2.0",
            "method": "notifications/initialized",
            "params": {}
        })
        .to_string();
        write_line(&mut self.writer, &notif).await?;
        tracing::debug!("relay: ensure_initialized done");

        self.initialized = true;
        Ok(())
    }
}

async fn write_line(writer: &mut (impl AsyncWrite + Unpin), line: &str) -> anyhow::Result<()> {
    tracing::trace!(len = line.len(), "relay: write_line {} bytes", line.len());
    writer.write_all(line.as_bytes()).await?;
    writer.write_all(b"\n").await?;
    writer.flush().await?;
    tracing::trace!("relay: write_line flushed");
    Ok(())
}

fn read_matching_response<'a, R: AsyncRead + Unpin>(
    reader: &'a mut BufReader<R>,
    expected_id: Option<Value>,
) -> impl std::future::Future<Output = anyhow::Result<String>> + 'a {
    async move {
        tracing::debug!(?expected_id, "relay: read_matching_response waiting for id");
        let mut line = String::new();
        loop {
            line.clear();
            tracing::trace!("relay: read_matching_response calling read_line");
            let n = reader.read_line(&mut line).await?;
            tracing::trace!(n, "relay: read_line returned {n} bytes");
            if n == 0 {
                tracing::warn!("relay: channel closed (EOF) while waiting for id={expected_id:?}");
                anyhow::bail!("relay channel closed unexpectedly");
            }
            let trimmed = line.trim();
            if trimmed.is_empty() {
                continue;
            }
            if let Ok(val) = serde_json::from_str::<Value>(trimmed) {
                let response_id = val.get("id").cloned();
                tracing::trace!(?response_id, ?expected_id, "relay: got line with id");
                if response_id == expected_id {
                    tracing::debug!(?expected_id, "relay: matched response id");
                    return Ok(trimmed.to_string());
                }
                tracing::debug!(
                    ?response_id,
                    ?expected_id,
                    "relay: discarding unmatched relay line"
                );
            } else {
                tracing::debug!(line = %trimmed, "relay: discarding non-JSON line");
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
