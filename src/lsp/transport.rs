use jsonrpsee::core::client::{ReceivedMessage, TransportReceiverT, TransportSenderT};
use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt, BufReader};
use tokio::sync::mpsc;
use tracing::{debug, trace, warn};

#[derive(thiserror::Error, Debug)]
pub enum TransportError {
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("parse error: {0}")]
    Parse(String),
}

// ── Writer task ──────────────────────────────────────────────────────

/// Background task that serialises all writes to the LSP stdin.
/// Both `Sender` (jsonrpsee outgoing) and `Receiver` (auto-responses)
/// send framed LSP messages through the same mpsc channel.  The writer
/// task drains them one at a time, so interleaving is impossible.
async fn writer_task<W: AsyncWrite + Send + Unpin + 'static>(
    mut writer: W,
    mut rx: mpsc::Receiver<String>,
) {
    while let Some(msg) = rx.recv().await {
        if let Err(e) = writer.write_all(msg.as_bytes()).await {
            warn!("transport writer task: write failed: {e}");
            break;
        }
        if let Err(e) = writer.flush().await {
            warn!("transport writer task: flush failed: {e}");
            break;
        }
    }
    debug!("transport writer task exiting");
}

// ── Sender ───────────────────────────────────────────────────────────

pub struct Sender(mpsc::Sender<String>);

#[async_trait::async_trait]
impl TransportSenderT for Sender {
    type Error = TransportError;

    async fn send(&mut self, msg: String) -> Result<(), Self::Error> {
        let framed = format!("Content-Length: {}\r\n\r\n{}", msg.len(), msg);
        debug!(msg_len = msg.len(), "transport sender: sending message");
        self.0
            .send(framed)
            .await
            .map_err(|_| TransportError::Parse("writer channel closed".into()))?;
        Ok(())
    }
}

// ── Receiver ─────────────────────────────────────────────────────────

pub struct Receiver<O: AsyncRead + Send + Sync + Unpin + 'static> {
    reader: BufReader<O>,
    write_tx: mpsc::Sender<String>,
}

impl<O: AsyncRead + Send + Sync + Unpin + 'static> Receiver<O> {
    /// Read one raw LSP message from the stream.
    async fn read_message(&mut self) -> Result<Vec<u8>, TransportError> {
        let mut content_length: Option<usize> = None;
        let mut line = String::new();

        loop {
            line.clear();
            let n = self.reader.read_line(&mut line).await?;
            if n == 0 {
                return Err(TransportError::Parse("unexpected EOF".into()));
            }
            let trimmed = line.trim();
            if trimmed.is_empty() {
                break;
            }
            if let Some(val) = trimmed.strip_prefix("Content-Length: ") {
                content_length = Some(
                    val.parse()
                        .map_err(|e| TransportError::Parse(format!("bad Content-Length: {e}")))?,
                );
            }
        }

        let len =
            content_length.ok_or_else(|| TransportError::Parse("missing Content-Length".into()))?;
        let mut buf = vec![0u8; len];
        self.reader.read_exact(&mut buf).await?;
        Ok(buf)
    }

    /// Send an auto-response for a server→client request through the writer channel.
    async fn send_response(
        &self,
        id: &serde_json::Value,
        result: serde_json::Value,
    ) -> Result<(), TransportError> {
        let resp = serde_json::json!({
            "jsonrpc": "2.0",
            "id": id,
            "result": result,
        });
        let body = resp.to_string();
        let framed = format!("Content-Length: {}\r\n\r\n{}", body.len(), body);
        self.write_tx
            .send(framed)
            .await
            .map_err(|_| TransportError::Parse("writer channel closed".into()))?;
        Ok(())
    }

    /// Check if a raw message is a server→client request (has both `id` and `method`)
    fn try_handle_server_request(buf: &[u8]) -> Option<ServerRequest> {
        let val: serde_json::Value = serde_json::from_slice(buf).ok()?;
        let id = val.get("id")?;
        let method = val.get("method")?.as_str()?;
        Some(ServerRequest {
            id: id.clone(),
            method: method.to_string(),
            params: val
                .get("params")
                .cloned()
                .unwrap_or(serde_json::Value::Null),
        })
    }
}

struct ServerRequest {
    id: serde_json::Value,
    method: String,
    params: serde_json::Value,
}

/// Build a default response for a known server→client request method.
fn default_response_for(method: &str, params: &serde_json::Value) -> serde_json::Value {
    match method {
        "workspace/configuration" => {
            let items = params
                .get("items")
                .and_then(|v| v.as_array())
                .cloned()
                .unwrap_or_default();
            let configs: Vec<serde_json::Value> = items
                .iter()
                .map(|item| {
                    let section = item.get("section").and_then(|s| s.as_str()).unwrap_or("");
                    match section {
                        "python" | "python.analysis" | "basedpyright" => serde_json::json!({
                            "analysis": {
                                "diagnosticMode": "openFilesOnly"
                            }
                        }),
                        _ => serde_json::json!({}),
                    }
                })
                .collect();
            serde_json::Value::Array(configs)
        }
        _ => serde_json::Value::Null,
    }
}

#[async_trait::async_trait]
impl<O: AsyncRead + Send + Sync + Unpin + 'static> TransportReceiverT for Receiver<O> {
    type Error = TransportError;

    async fn receive(&mut self) -> Result<ReceivedMessage, Self::Error> {
        loop {
            trace!("transport receiver: waiting for next message");
            let buf = self.read_message().await?;

            if let Some(req) = Self::try_handle_server_request(&buf) {
                let result = default_response_for(&req.method, &req.params);
                debug!(
                    method = %req.method, id = %req.id,
                    params = %req.params, result = %result,
                    "auto-responding to server request"
                );
                if let Err(e) = self.send_response(&req.id, result).await {
                    warn!(method = %req.method, "failed to auto-respond: {e}");
                }
                continue;
            }

            let preview = String::from_utf8_lossy(&buf[..buf.len().min(200)]);
            debug!(len = buf.len(), preview = %preview, "transport receiver: passing message to jsonrpsee");
            return Ok(ReceivedMessage::Bytes(buf));
        }
    }
}

// ── Constructor ──────────────────────────────────────────────────────

const WRITE_CHANNEL_SIZE: usize = 64;

pub async fn io_transport<I, O>(input: I, output: O) -> (Sender, Receiver<O>)
where
    I: AsyncWrite + Send + Unpin + 'static,
    O: AsyncRead + Send + Sync + Unpin + 'static,
{
    let (tx, rx) = mpsc::channel::<String>(WRITE_CHANNEL_SIZE);
    tokio::spawn(writer_task(input, rx));
    (
        Sender(tx.clone()),
        Receiver {
            reader: BufReader::new(output),
            write_tx: tx,
        },
    )
}
