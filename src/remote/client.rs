use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use rmcp::{
    model::{
        CallToolRequestParams, CallToolResult, Content, Implementation, ListToolsResult,
        PaginatedRequestParams, ServerCapabilities, ServerInfo,
    },
    ErrorData as McpError, RoleServer, ServerHandler, ServiceExt,
};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::unix::{OwnedReadHalf, OwnedWriteHalf};
use tokio::net::UnixStream;
use tokio::process::Command;
use tokio::sync::Mutex;
use tracing::{debug, info, warn};

use crate::config::Config;
use crate::relay::{self, RelayChannel};

/// Parsed remote spec: [user@]host[:port]
struct RemoteSpec {
    user: Option<String>,
    host: String,
    port: Option<u16>,
}

impl RemoteSpec {
    fn parse(s: &str) -> anyhow::Result<Self> {
        let (user, rest) = if let Some((u, r)) = s.split_once('@') {
            (Some(u.to_string()), r)
        } else {
            (None, s)
        };

        let (host, port) = if let Some((h, p)) = rest.rsplit_once(':') {
            let port: u16 = p
                .parse()
                .map_err(|_| anyhow::anyhow!("invalid port: {p}"))?;
            (h.to_string(), Some(port))
        } else {
            (rest.to_string(), None)
        };

        Ok(Self { user, host, port })
    }

    fn ssh_base_args(&self) -> Vec<String> {
        let mut args = Vec::new();
        if let Some(port) = self.port {
            args.extend(["-p".to_string(), port.to_string()]);
        }
        let dest = if let Some(ref user) = self.user {
            format!("{user}@{}", self.host)
        } else {
            self.host.clone()
        };
        args.push(dest);
        args
    }
}

// ── Connection state ──────────────────────────────────────────────────────────

/// Holds everything needed to reconnect to the remote.
struct ConnectionParams {
    spec: RemoteSpec,
    remote_control: String,
    local_dir: tempfile::TempDir,
}

struct ActiveConnection {
    relay: RelayChannel<OwnedWriteHalf, OwnedReadHalf>,
    session_ssh: tokio::process::Child,
}

impl Drop for ActiveConnection {
    fn drop(&mut self) {
        // Best-effort kill; ignore errors (process may have already exited).
        self.session_ssh.start_kill().ok();
    }
}

impl ConnectionParams {
    /// Establish a new session and return an active connection.
    async fn connect(&self) -> anyhow::Result<ActiveConnection> {
        // Forward control socket
        let local_control = self.local_dir.path().join("ctrl.sock");
        let _ = std::fs::remove_file(&local_control);
        let mut control_ssh = start_ssh_forward(&self.spec, &local_control, &self.remote_control)?;

        if !wait_for_socket(&local_control).await {
            control_ssh.kill().await.ok();
            anyhow::bail!("timeout waiting for control socket tunnel");
        }

        // Establish session
        let session_id = generate_session_id();
        let remote_session_path = establish_session(&local_control, &session_id).await?;
        control_ssh.kill().await.ok();

        info!(session = %session_id, "session established");

        // Forward session socket
        let local_session = self.local_dir.path().join("sess.sock");
        let _ = std::fs::remove_file(&local_session);
        let session_ssh = start_ssh_forward(&self.spec, &local_session, &remote_session_path)?;

        if !wait_for_socket(&local_session).await {
            anyhow::bail!("timeout waiting for session socket tunnel");
        }

        // Connect and create relay
        let stream = UnixStream::connect(&local_session).await?;
        let (read_half, write_half) = stream.into_split();
        let relay = RelayChannel::new(write_half, read_half);

        Ok(ActiveConnection { relay, session_ssh })
    }

    /// Try to reconnect, retrying once per second for up to 30 seconds.
    async fn reconnect(&self) -> anyhow::Result<ActiveConnection> {
        for attempt in 1..=30 {
            debug!(attempt, "attempting reconnect");
            match self.connect().await {
                Ok(conn) => {
                    info!(attempt, "reconnected to remote");
                    return Ok(conn);
                }
                Err(e) => {
                    debug!(attempt, error = %e, "reconnect attempt failed");
                    tokio::time::sleep(std::time::Duration::from_secs(1)).await;
                }
            }
        }
        anyhow::bail!("failed to reconnect after 30 attempts")
    }
}

// ── MCP proxy server ──────────────────────────────────────────────────────────

/// Shared setup state: `None` = still connecting, `Some(Ok)` = ready,
/// `Some(Err(msg))` = failed.
type SetupState = Arc<std::sync::Mutex<Option<Result<(), String>>>>;

#[derive(Clone)]
struct RemoteProxyServer {
    /// The live connection; `None` while SSH setup is still running.
    conn: Arc<Mutex<Option<ActiveConnection>>>,
    /// Connection parameters for reconnection; `None` while SSH setup is still running.
    conn_params: Arc<Mutex<Option<Arc<ConnectionParams>>>>,
    /// Signals when SSH setup has finished (success or failure).
    setup_watch: tokio::sync::watch::Receiver<bool>,
    /// Error message if setup failed.
    setup_state: SetupState,
    next_id: Arc<AtomicU64>,
}

impl RemoteProxyServer {
    /// Block until the initial SSH setup completes (or fails / times out).
    async fn wait_for_setup(&self) -> Result<(), McpError> {
        // Fast path: already done.
        if *self.setup_watch.borrow() {
            return self.check_setup_error();
        }

        let mut rx = self.setup_watch.clone();
        let timed_out = tokio::time::timeout(Duration::from_secs(60), rx.wait_for(|v| *v))
            .await
            .is_err();
        drop(rx);
        if timed_out {
            return Err(McpError::internal_error(
                "timed out waiting for SSH connection (60s)",
                None,
            ));
        }
        self.check_setup_error()
    }

    fn check_setup_error(&self) -> Result<(), McpError> {
        if let Some(Err(msg)) = self.setup_state.lock().unwrap().as_ref() {
            return Err(McpError::internal_error(
                format!("SSH setup failed: {msg}"),
                None,
            ));
        }
        Ok(())
    }

    /// Relay a JSON-RPC request, reconnecting on failure.
    async fn relay_with_reconnect(&self, req_json: &str) -> Result<String, McpError> {
        info!("relay_with_reconnect: waiting for setup");
        self.wait_for_setup().await?;
        info!("relay_with_reconnect: setup ready, acquiring conn lock");

        // First attempt
        {
            let mut guard = self.conn.lock().await;
            info!(
                "relay_with_reconnect: conn lock acquired, conn present={}",
                guard.is_some()
            );
            if let Some(conn) = guard.as_mut() {
                info!(
                    "relay_with_reconnect: calling relay.relay (initialized={})",
                    conn.relay.is_initialized()
                );
                match conn.relay.relay(req_json).await {
                    Ok(raw) => {
                        info!("relay_with_reconnect: relay succeeded");
                        return Ok(raw);
                    }
                    Err(e) => {
                        warn!("relay failed, attempting reconnect: {e}");
                    }
                }
            }
        }

        // Reconnect
        info!("relay_with_reconnect: attempting reconnect");
        let params_guard = self.conn_params.lock().await;
        let params = params_guard.as_ref().ok_or_else(|| {
            McpError::internal_error("no connection params available for reconnect", None)
        })?;
        let new_conn = params
            .reconnect()
            .await
            .map_err(|e| McpError::internal_error(format!("reconnect failed: {e}"), None))?;
        drop(params_guard);

        info!("relay_with_reconnect: reconnected, retrying request");
        let mut guard = self.conn.lock().await;
        *guard = Some(new_conn);

        // Retry the request
        guard
            .as_mut()
            .unwrap()
            .relay
            .relay(req_json)
            .await
            .map_err(|e| {
                McpError::internal_error(format!("relay failed after reconnect: {e}"), None)
            })
    }
}

impl ServerHandler for RemoteProxyServer {
    fn get_info(&self) -> ServerInfo {
        ServerInfo::new(ServerCapabilities::builder().enable_tools().build())
            .with_server_info(Implementation::new(
                "programmer-mcp-remote",
                env!("CARGO_PKG_VERSION"),
            ))
            .with_instructions(
                "Remote proxy to a programmer-mcp instance. All tool calls are forwarded \
                 to the remote server. Automatically reconnects on server restart.",
            )
    }

    async fn call_tool(
        &self,
        request: CallToolRequestParams,
        _context: rmcp::service::RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let params = serde_json::json!({
            "name": request.name,
            "arguments": request.arguments.unwrap_or_default(),
        });
        let req_json = relay::build_jsonrpc_request(id, "tools/call", params);

        let raw = self.relay_with_reconnect(&req_json).await?;
        extract_call_result(&raw)
    }

    async fn list_tools(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: rmcp::service::RequestContext<RoleServer>,
    ) -> Result<ListToolsResult, McpError> {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let req_json = relay::build_jsonrpc_request(id, "tools/list", serde_json::json!({}));

        let raw = self.relay_with_reconnect(&req_json).await?;

        let val: serde_json::Value = serde_json::from_str(&raw).map_err(|e| {
            McpError::internal_error(format!("failed to parse remote response: {e}"), None)
        })?;

        let result = val.get("result").ok_or_else(|| {
            McpError::internal_error("missing result in remote tools/list response", None)
        })?;

        serde_json::from_value(result.clone()).map_err(|e| {
            McpError::internal_error(format!("failed to deserialize ListToolsResult: {e}"), None)
        })
    }
}

fn extract_call_result(raw: &str) -> Result<CallToolResult, McpError> {
    let val: serde_json::Value = serde_json::from_str(raw).map_err(|e| {
        McpError::internal_error(format!("failed to parse remote response: {e}"), None)
    })?;

    if let Some(err) = val.get("error") {
        let msg = err["message"]
            .as_str()
            .unwrap_or("unknown error from remote");
        return Ok(CallToolResult::error(vec![Content::text(msg)]));
    }

    let result = val
        .get("result")
        .ok_or_else(|| McpError::internal_error("missing result in remote response", None))?;

    serde_json::from_value(result.clone()).map_err(|e| {
        McpError::internal_error(
            format!("failed to deserialize CallToolResult from remote: {e}"),
            None,
        )
    })
}

// ── entry point ───────────────────────────────────────────────────────────────

pub async fn run_remote_client(config: Config) -> anyhow::Result<()> {
    let remote_str = config.remote.as_deref().unwrap().to_string();
    let debug_mode = config.debug;

    // Shared state populated by the background setup task.
    let conn: Arc<Mutex<Option<ActiveConnection>>> = Arc::new(Mutex::new(None));
    let conn_params: Arc<Mutex<Option<Arc<ConnectionParams>>>> = Arc::new(Mutex::new(None));
    let setup_state: SetupState = Arc::new(std::sync::Mutex::new(None));
    let (setup_tx, setup_rx) = tokio::sync::watch::channel(false);

    // Spawn SSH setup in the background so stdio is available immediately.
    {
        let conn = conn.clone();
        let conn_params = conn_params.clone();
        let setup_state = setup_state.clone();
        tokio::spawn(async move {
            let result: anyhow::Result<()> = async {
                let spec = RemoteSpec::parse(&remote_str)?;
                let remote_control = find_remote_socket(&spec, debug_mode).await?;
                info!(socket = %remote_control, "found remote control socket");

                let local_dir = tempfile::tempdir()?;
                let params = Arc::new(ConnectionParams {
                    spec,
                    remote_control,
                    local_dir,
                });

                let initial_conn = params.connect().await?;
                info!("initial connection ready");

                *conn_params.lock().await = Some(params);
                *conn.lock().await = Some(initial_conn);
                Ok(())
            }
            .await;

            let outcome = result.map_err(|e| e.to_string());
            if let Err(ref msg) = outcome {
                tracing::error!("SSH setup failed: {msg}");
            }
            *setup_state.lock().unwrap() = Some(outcome);
            // Notify waiters regardless of success/failure.
            setup_tx.send(true).ok();
        });
    }

    let proxy = RemoteProxyServer {
        conn,
        conn_params,
        setup_watch: setup_rx,
        setup_state,
        next_id: Arc::new(AtomicU64::new(1)),
    };

    let service = proxy
        .serve(rmcp::transport::stdio())
        .await
        .inspect_err(|e| {
            tracing::error!("remote proxy serve error: {e:?}");
        })?;

    service.waiting().await?;
    info!("remote proxy shut down");

    Ok(())
}

// ── SSH helpers ───────────────────────────────────────────────────────────────

async fn establish_session(
    local_control: &std::path::Path,
    session_id: &str,
) -> anyhow::Result<String> {
    let stream = UnixStream::connect(local_control).await?;
    let (reader, mut writer) = stream.into_split();

    writer
        .write_all(format!("SESSION {session_id}\n").as_bytes())
        .await?;
    writer.shutdown().await?;

    let mut lines = BufReader::new(reader);
    let mut response = String::new();
    lines.read_line(&mut response).await?;
    let response = response.trim();

    let parts: Vec<&str> = response.splitn(3, ' ').collect();
    if parts.len() != 3 || parts[0] != "OK" {
        anyhow::bail!("unexpected session response: {response}");
    }

    Ok(parts[2].to_string())
}

fn start_ssh_forward(
    spec: &RemoteSpec,
    local_path: &std::path::Path,
    remote_path: &str,
) -> anyhow::Result<tokio::process::Child> {
    let mut args = spec.ssh_base_args();
    args.extend([
        "-N".to_string(),
        "-L".to_string(),
        format!("{}:{}", local_path.display(), remote_path),
    ]);

    debug!(args = ?args, "starting SSH tunnel");

    let child = Command::new("ssh")
        .args(&args)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true)
        .spawn()?;

    Ok(child)
}

async fn find_remote_socket(spec: &RemoteSpec, debug_mode: bool) -> anyhow::Result<String> {
    // Resolve ~ to the actual home directory on the remote
    let socket_dir = ssh_command(spec, "echo -n ~/.local/share/programmer-mcp").await?;
    let socket_dir = socket_dir.trim();

    if debug_mode {
        return Ok(format!("{socket_dir}/debug-mcp.sock"));
    }

    let output = ssh_command(
        spec,
        &format!("ls {socket_dir}/*.sock 2>/dev/null | grep -v debug-mcp | grep -v '.session-'"),
    )
    .await?;

    let sockets: Vec<&str> = output.lines().filter(|l| !l.is_empty()).collect();

    match sockets.len() {
        0 => anyhow::bail!("no programmer-mcp instances found on remote"),
        1 => Ok(sockets[0].to_string()),
        _ => {
            eprintln!("Multiple programmer-mcp instances found on remote:");
            for (i, s) in sockets.iter().enumerate() {
                eprintln!("  [{}] {}", i + 1, s);
            }
            eprint!("Choose [1-{}]: ", sockets.len());

            let mut input = String::new();
            std::io::stdin().read_line(&mut input)?;
            let choice: usize = input
                .trim()
                .parse()
                .map_err(|_| anyhow::anyhow!("invalid choice"))?;

            if choice == 0 || choice > sockets.len() {
                anyhow::bail!("choice out of range");
            }

            Ok(sockets[choice - 1].to_string())
        }
    }
}

async fn ssh_command(spec: &RemoteSpec, command: &str) -> anyhow::Result<String> {
    let mut args = spec.ssh_base_args();
    args.push(command.to_string());

    let output = Command::new("ssh").args(&args).output().await?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!("SSH command failed: {stderr}");
    }

    Ok(String::from_utf8_lossy(&output.stdout).to_string())
}

fn generate_session_id() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let ts = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_millis();
    format!("{ts:x}")
}

async fn wait_for_socket(path: &std::path::Path) -> bool {
    for _ in 0..60 {
        if path.exists() {
            return true;
        }
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    }
    false
}
