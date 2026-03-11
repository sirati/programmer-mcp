use std::time::Duration;

use serde_json::Value;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{ChildStdin, ChildStdout};

const PROTOCOL_VERSION: &str = "2025-06-18";
const INIT_TIMEOUT_SECS: u64 = 10;

pub struct RelayChannel {
    pub stdin: ChildStdin,
    pub stdout: BufReader<ChildStdout>,
    initialized: bool,
}

impl RelayChannel {
    pub fn new(stdin: ChildStdin, stdout: ChildStdout) -> Self {
        Self {
            stdin,
            stdout: BufReader::new(stdout),
            initialized: false,
        }
    }

    pub async fn relay(&mut self, request_json: &str) -> anyhow::Result<String> {
        if !self.initialized {
            self.ensure_initialized().await?;
        }
        write_request(&mut self.stdin, request_json).await?;
        let expected_id = extract_id(request_json);
        tokio::time::timeout(
            Duration::from_secs(30),
            read_matching_response(&mut self.stdout, expected_id),
        )
        .await
        .map_err(|_| anyhow::anyhow!("relay timed out waiting for response"))?
    }

    pub async fn ensure_initialized(&mut self) -> anyhow::Result<()> {
        let init_req = serde_json::json!({
            "jsonrpc": "2.0",
            "id": 0,
            "method": "initialize",
            "params": {
                "protocolVersion": PROTOCOL_VERSION,
                "capabilities": {},
                "clientInfo": {
                    "name": "debug-relay",
                    "version": env!("CARGO_PKG_VERSION")
                }
            }
        })
        .to_string();

        write_request(&mut self.stdin, &init_req).await?;

        tokio::time::timeout(
            Duration::from_secs(INIT_TIMEOUT_SECS),
            read_matching_response(&mut self.stdout, Some(serde_json::json!(0))),
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
        write_request(&mut self.stdin, &notif).await?;

        self.initialized = true;
        Ok(())
    }
}

async fn write_request(stdin: &mut ChildStdin, request_json: &str) -> anyhow::Result<()> {
    stdin.write_all(request_json.as_bytes()).await?;
    stdin.write_all(b"\n").await?;
    stdin.flush().await?;
    Ok(())
}

async fn read_matching_response(
    stdout: &mut BufReader<ChildStdout>,
    expected_id: Option<Value>,
) -> anyhow::Result<String> {
    let mut line = String::new();
    loop {
        line.clear();
        let n = stdout.read_line(&mut line).await?;
        if n == 0 {
            anyhow::bail!("child stdout closed unexpectedly");
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
            // Notification or unrelated response — discard and keep reading.
            tracing::debug!("discarding unmatched child stdout line");
        }
    }
}

fn extract_id(json: &str) -> Option<Value> {
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
