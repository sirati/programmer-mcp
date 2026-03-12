//! SSH connection state types and connection logic.

use tokio::net::unix::{OwnedReadHalf, OwnedWriteHalf};
use tokio::net::UnixStream;
use tracing::{debug, info};

use crate::relay::RelayChannel;

use super::ssh::{establish_session, generate_session_id, start_ssh_forward, wait_for_socket};

// ── RemoteSpec ────────────────────────────────────────────────────────────────

/// Parsed remote spec: [user@]host[:port]
pub struct RemoteSpec {
    pub user: Option<String>,
    pub host: String,
    pub port: Option<u16>,
}

impl RemoteSpec {
    pub fn parse(s: &str) -> anyhow::Result<Self> {
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

    pub fn ssh_base_args(&self) -> Vec<String> {
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

// ── ActiveConnection ──────────────────────────────────────────────────────────

pub struct ActiveConnection {
    pub relay: RelayChannel<OwnedWriteHalf, OwnedReadHalf>,
    pub session_ssh: tokio::process::Child,
}

impl Drop for ActiveConnection {
    fn drop(&mut self) {
        // Best-effort kill; ignore errors (process may have already exited).
        self.session_ssh.start_kill().ok();
    }
}

// ── ConnectionParams ──────────────────────────────────────────────────────────

/// Holds everything needed to establish and re-establish a remote connection.
pub struct ConnectionParams {
    pub spec: RemoteSpec,
    pub remote_control: String,
    pub local_dir: tempfile::TempDir,
}

impl ConnectionParams {
    /// Establish a new session and return an active connection.
    pub async fn connect(&self) -> anyhow::Result<ActiveConnection> {
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
    pub async fn reconnect(&self) -> anyhow::Result<ActiveConnection> {
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
