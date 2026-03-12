use std::path::{Path, PathBuf};

use rmcp::{ServerHandler, ServiceExt};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::{UnixListener, UnixStream};
use tokio::task::JoinHandle;
use tracing::{debug, error, info, warn};

/// Listens on a Unix socket for remote session requests.
/// Each accepted session gets a single bidirectional Unix socket
/// and a cloned server instance serving MCP over it.
///
/// Generic over any `ServerHandler + Clone + Send + 'static` server type.
pub struct RemoteListener {
    control_path: PathBuf,
    socket_dir: PathBuf,
    task: Option<JoinHandle<()>>,
}

impl RemoteListener {
    pub fn new(control_path: PathBuf) -> Self {
        let socket_dir = control_path.parent().unwrap().to_path_buf();
        Self {
            control_path,
            socket_dir,
            task: None,
        }
    }

    /// Start listening for remote session requests. This spawns a background task.
    /// `server` is cloned for each new session.
    pub fn start<S>(&mut self, server: S)
    where
        S: ServerHandler + Clone + Send + Sync + 'static,
    {
        let control_path = self.control_path.clone();
        let socket_dir = self.socket_dir.clone();
        let handle = tokio::spawn(async move {
            if let Err(e) = listen_loop(&control_path, &socket_dir, server).await {
                error!("remote listener error: {e}");
            }
        });
        self.task = Some(handle);
    }

    /// Shut down the listener and abort any spawned tasks.
    pub fn shutdown(&mut self) {
        if let Some(handle) = self.task.take() {
            handle.abort();
        }
        let _ = std::fs::remove_file(&self.control_path);
    }
}

impl Drop for RemoteListener {
    fn drop(&mut self) {
        self.shutdown();
    }
}

async fn listen_loop<S>(control_path: &Path, socket_dir: &Path, server: S) -> anyhow::Result<()>
where
    S: ServerHandler + Clone + Send + Sync + 'static,
{
    // Ensure socket directory exists
    std::fs::create_dir_all(socket_dir)?;

    // Clean up stale control socket
    let _ = std::fs::remove_file(control_path);

    let listener = UnixListener::bind(control_path)?;
    info!(path = %control_path.display(), "remote control socket listening");

    loop {
        match listener.accept().await {
            Ok((stream, _)) => {
                let srv = server.clone();
                let sd = socket_dir.to_path_buf();
                tokio::spawn(async move {
                    if let Err(e) = handle_session_request(stream, &sd, srv).await {
                        warn!("session request error: {e}");
                    }
                });
            }
            Err(e) => {
                warn!("remote accept error: {e}");
            }
        }
    }
}

/// Handle a single session negotiation on the control socket.
///
/// Protocol:
///   Client sends: `SESSION <random_string>\n`
///   Server creates a session socket and responds: `OK <session_id> <socket_path>\n`
///   Client then connects to the session socket (possibly via SSH forwarding).
///   Server serves MCP on that bidirectional socket.
async fn handle_session_request<S>(
    stream: UnixStream,
    socket_dir: &Path,
    server: S,
) -> anyhow::Result<()>
where
    S: ServerHandler + Send + Sync + 'static,
{
    let (reader, mut writer) = stream.into_split();
    let mut lines = BufReader::new(reader);
    let mut line = String::new();
    lines.read_line(&mut line).await?;
    let line = line.trim();

    let session_id = if let Some(id) = line.strip_prefix("SESSION ") {
        id.trim().to_string()
    } else if line == "LIST" {
        let sockets = list_available_sockets(socket_dir)?;
        let response = format!("SOCKETS {}\n", sockets.join(" "));
        writer.write_all(response.as_bytes()).await?;
        return Ok(());
    } else {
        anyhow::bail!("invalid session request: {line}");
    };

    // Use /tmp for session sockets to avoid SUN_LEN path length limits (~108 chars).
    let session_dir = std::env::temp_dir().join("programmer-mcp-sessions");
    std::fs::create_dir_all(&session_dir)?;
    let session_path = session_dir.join(format!("{session_id}.sock"));

    // Clean up any stale socket
    let _ = std::fs::remove_file(&session_path);

    // Bind the session socket BEFORE responding
    let session_listener = UnixListener::bind(&session_path)?;

    debug!(session = %session_id, "session socket created");

    // Tell the client the session is ready
    let response = format!("OK {} {}\n", session_id, session_path.display());
    writer.write_all(response.as_bytes()).await?;
    writer.shutdown().await?;

    // Wait for client to connect (with timeout)
    let timeout = std::time::Duration::from_secs(30);
    let (session_stream, _) = tokio::time::timeout(timeout, session_listener.accept())
        .await
        .map_err(|_| anyhow::anyhow!("timeout waiting for client on session socket"))?
        .map_err(|e| anyhow::anyhow!("accept error on session socket: {e}"))?;

    info!(session = %session_id, "remote session connected");

    // Serve MCP on the bidirectional socket
    let (read_half, write_half) = tokio::io::split(session_stream);
    let service = server
        .serve((read_half, write_half))
        .await
        .map_err(|e| anyhow::anyhow!("failed to serve remote session {session_id}: {e}"))?;

    // Run in background - when the session ends, clean up
    let sp = session_path.clone();
    let sid = session_id.clone();
    tokio::spawn(async move {
        let _ = service.waiting().await;
        info!(session = %sid, "remote session ended");
        let _ = std::fs::remove_file(&sp);
    });

    Ok(())
}

/// List non-debug .sock files in the socket directory (for session selection).
fn list_available_sockets(socket_dir: &Path) -> anyhow::Result<Vec<String>> {
    let mut sockets = Vec::new();
    if let Ok(entries) = std::fs::read_dir(socket_dir) {
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().to_string();
            if name.ends_with(".sock") && !name.contains(".session-") && name != "debug-mcp.sock" {
                sockets.push(name);
            }
        }
    }
    Ok(sockets)
}
