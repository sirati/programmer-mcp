//! Remote proxy MCP client.
//!
//! Connects to a remote `programmer-mcp` instance via SSH tunnels and proxies
//! all tool calls to it.  SSH setup runs in the background so stdio is available
//! immediately; tool calls block until the tunnel is ready.

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
use tokio::sync::Mutex;
use tracing::{info, warn};

use crate::config::Config;
use crate::relay;

use super::connection::{ActiveConnection, ConnectionParams};
use super::ssh::find_remote_socket;

pub use super::connection::RemoteSpec;

// ── Shared setup state ────────────────────────────────────────────────────────

/// `None` = still connecting, `Some(Ok)` = ready, `Some(Err(msg))` = failed.
type SetupState = Arc<std::sync::Mutex<Option<Result<(), String>>>>;

// ── RemoteProxyServer ─────────────────────────────────────────────────────────

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

    /// Relay a JSON-RPC request to the remote, reconnecting on failure.
    async fn relay_with_reconnect(&self, req_json: &str) -> Result<String, McpError> {
        self.wait_for_setup().await?;

        // First attempt
        {
            let mut guard = self.conn.lock().await;
            if let Some(conn) = guard.as_mut() {
                match conn.relay.relay(req_json).await {
                    Ok(raw) => return Ok(raw),
                    Err(e) => warn!("relay failed, attempting reconnect: {e}"),
                }
            }
        }

        // Reconnect
        let params_guard = self.conn_params.lock().await;
        let params = params_guard.as_ref().ok_or_else(|| {
            McpError::internal_error("no connection params available for reconnect", None)
        })?;
        let new_conn = params
            .reconnect()
            .await
            .map_err(|e| McpError::internal_error(format!("reconnect failed: {e}"), None))?;
        drop(params_guard);

        let mut guard = self.conn.lock().await;
        *guard = Some(new_conn);

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

// ── entry point ───────────────────────────────────────────────────────────────

pub async fn run_remote_client(config: Config) -> anyhow::Result<()> {
    let remote_str = config.remote.as_deref().unwrap().to_string();
    let debug_mode = config.debug;

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
        .inspect_err(|e| tracing::error!("remote proxy serve error: {e:?}"))?;

    service.waiting().await?;
    info!("remote proxy shut down");
    Ok(())
}

// ── helpers ───────────────────────────────────────────────────────────────────

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
