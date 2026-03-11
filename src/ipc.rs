use std::path::{Path, PathBuf};
use std::sync::Arc;

use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::{UnixListener, UnixStream};
use tokio::sync::{watch, Mutex};
use tracing::{debug, error, info, warn};

/// Manages a Unix domain socket for receiving human messages.
pub struct HumanMessageBus {
    socket_path: PathBuf,
    pending: Arc<Mutex<Vec<String>>>,
    notify: watch::Sender<()>,
    wait: watch::Receiver<()>,
}

impl HumanMessageBus {
    /// Create a new bus and start listening on `{workspace}/.programmer-mcp.sock`.
    pub fn start(workspace: &Path) -> Arc<Self> {
        let socket_path = workspace.join(".programmer-mcp.sock");

        // Clean up stale socket
        let _ = std::fs::remove_file(&socket_path);

        let (notify, wait) = watch::channel(());
        let bus = Arc::new(Self {
            socket_path: socket_path.clone(),
            pending: Arc::new(Mutex::new(Vec::new())),
            notify,
            wait,
        });

        let bus_clone = bus.clone();
        tokio::spawn(async move {
            if let Err(e) = bus_clone.listen_loop().await {
                error!("IPC listener error: {e}");
            }
        });

        bus
    }

    async fn listen_loop(&self) -> anyhow::Result<()> {
        let listener = UnixListener::bind(&self.socket_path)?;
        info!(path = %self.socket_path.display(), "IPC socket listening");

        loop {
            match listener.accept().await {
                Ok((stream, _)) => {
                    let pending = self.pending.clone();
                    let notify = self.notify.clone();
                    tokio::spawn(async move {
                        if let Err(e) = handle_client(stream, pending, notify).await {
                            debug!("IPC client error: {e}");
                        }
                    });
                }
                Err(e) => {
                    warn!("IPC accept error: {e}");
                }
            }
        }
    }

    /// Block until a human message arrives, then return it.
    pub async fn wait_for_message(&self) -> String {
        loop {
            {
                let mut msgs = self.pending.lock().await;
                if !msgs.is_empty() {
                    return msgs.remove(0);
                }
            }
            let mut rx = self.wait.clone();
            let _ = rx.changed().await;
        }
    }

    /// Take all pending messages without blocking. Returns empty vec if none.
    pub async fn take_pending(&self) -> Vec<String> {
        let mut msgs = self.pending.lock().await;
        std::mem::take(&mut *msgs)
    }

    /// Get the socket path for display/logging.
    pub fn socket_path(&self) -> &Path {
        &self.socket_path
    }
}

impl Drop for HumanMessageBus {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.socket_path);
    }
}

async fn handle_client(
    stream: UnixStream,
    pending: Arc<Mutex<Vec<String>>>,
    notify: watch::Sender<()>,
) -> anyhow::Result<()> {
    let (reader, mut writer) = stream.into_split();
    let mut lines = BufReader::new(reader).lines();

    let mut message = String::new();
    while let Some(line) = lines.next_line().await? {
        if !message.is_empty() {
            message.push('\n');
        }
        message.push_str(&line);
    }

    if !message.is_empty() {
        debug!(len = message.len(), "received human message");
        pending.lock().await.push(message);
        let _ = notify.send(());
        writer.write_all(b"ok\n").await?;
    }

    Ok(())
}

/// Send a message to the running programmer-mcp instance via Unix socket.
pub async fn send_message(workspace: &Path, message: &str) -> anyhow::Result<()> {
    let socket_path = workspace.join(".programmer-mcp.sock");
    let stream = UnixStream::connect(&socket_path).await.map_err(|e| {
        anyhow::anyhow!(
            "cannot connect to {} — is programmer-mcp running? {e}",
            socket_path.display()
        )
    })?;

    let (reader, mut writer) = stream.into_split();
    writer.write_all(message.as_bytes()).await?;
    writer.shutdown().await?;

    // Wait for ack
    let mut buf = String::new();
    BufReader::new(reader).read_line(&mut buf).await?;

    Ok(())
}
