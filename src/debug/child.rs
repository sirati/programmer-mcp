use std::collections::VecDeque;
use std::path::Path;
use std::process::ExitStatus;
use std::sync::Arc;
use std::time::{Duration, Instant};

use tokio::process::{Child, Command};
use tokio::sync::Mutex;

use super::relay::RelayChannel;

const LOG_BUFFER_SIZE: usize = 2000;
const READY_CHECK_INTERVAL_MS: u64 = 300;
const READY_CHECK_ATTEMPTS: u32 = 10; // 3 seconds total

/// Describes why a child process exited.
pub struct ExitInfo {
    pub status: ExitStatus,
}

impl ExitInfo {
    pub fn describe(&self) -> String {
        if let Some(code) = self.status.code() {
            return format!("exited with code {code}");
        }
        #[cfg(unix)]
        {
            use std::os::unix::process::ExitStatusExt;
            if let Some(sig) = self.status.signal() {
                return format!("killed by signal {sig}");
            }
        }
        format!("exited abnormally ({})", self.status)
    }
}

pub struct ChildHandle {
    relay: Mutex<RelayChannel>,
    pub log_buffer: Arc<Mutex<VecDeque<String>>>,
    process: Mutex<Child>,
    pub started_at: Instant,
}

impl ChildHandle {
    pub async fn spawn(binary: &Path, args: &[String], workspace: &Path) -> anyhow::Result<Self> {
        let mut child = Command::new(binary)
            .args(args)
            .current_dir(workspace)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()?;

        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| anyhow::anyhow!("no stdin"))?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| anyhow::anyhow!("no stdout"))?;
        let stderr = child
            .stderr
            .take()
            .ok_or_else(|| anyhow::anyhow!("no stderr"))?;

        let log_buffer = Arc::new(Mutex::new(VecDeque::with_capacity(LOG_BUFFER_SIZE)));
        spawn_log_reader(stderr, log_buffer.clone());

        Ok(Self {
            relay: Mutex::new(RelayChannel::new(stdin, stdout)),
            log_buffer,
            process: Mutex::new(child),
            started_at: Instant::now(),
        })
    }

    pub async fn is_alive(&self) -> bool {
        let mut proc = self.process.lock().await;
        matches!(proc.try_wait(), Ok(None))
    }

    pub async fn kill(&self) {
        let mut proc = self.process.lock().await;
        let _ = proc.kill().await;
    }

    /// Wait up to ~3s checking every 300ms.
    /// Returns `Ok(())` if the process is still alive, or `Err(ExitInfo)` if it exited.
    pub async fn wait_for_ready(&self) -> Result<(), ExitInfo> {
        for _ in 0..READY_CHECK_ATTEMPTS {
            tokio::time::sleep(Duration::from_millis(READY_CHECK_INTERVAL_MS)).await;
            let mut proc = self.process.lock().await;
            if let Ok(Some(status)) = proc.try_wait() {
                return Err(ExitInfo { status });
            }
        }
        Ok(())
    }

    pub async fn relay(&self, request_json: &str) -> anyhow::Result<String> {
        let mut channel = self.relay.lock().await;
        channel.relay(request_json).await
    }

    /// Eagerly perform the MCP initialize handshake so callers can detect a
    /// broken child before entering proxy mode.
    pub async fn ensure_mcp_initialized(&self) -> anyhow::Result<()> {
        let mut channel = self.relay.lock().await;
        channel.ensure_initialized().await
    }

    pub async fn search_logs(&self, query: Option<&str>, limit: usize) -> Vec<String> {
        let buf = self.log_buffer.lock().await;
        buf.iter()
            .filter(|line| query.map_or(true, |q| line.contains(q)))
            .cloned()
            .rev()
            .take(limit)
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect()
    }
}

fn spawn_log_reader(stderr: tokio::process::ChildStderr, log_buffer: Arc<Mutex<VecDeque<String>>>) {
    use tokio::io::{AsyncBufReadExt, BufReader};

    tokio::spawn(async move {
        let mut reader = BufReader::new(stderr);
        let mut line = String::new();
        loop {
            line.clear();
            match reader.read_line(&mut line).await {
                Ok(0) | Err(_) => break,
                Ok(_) => {
                    let trimmed = line.trim_end().to_string();
                    tracing::debug!(target: "child", "{trimmed}");
                    let mut buf = log_buffer.lock().await;
                    if buf.len() >= LOG_BUFFER_SIZE {
                        buf.pop_front();
                    }
                    buf.push_back(trimmed);
                }
            }
        }
    });
}
