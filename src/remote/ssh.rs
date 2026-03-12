//! SSH helper functions for remote connections.

use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixStream;
use tokio::process::Command;
use tracing::debug;

use super::connection::RemoteSpec;

/// Establish a session via the control socket, returning the remote session socket path.
pub async fn establish_session(
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

/// Spawn an SSH tunnel forwarding `local_path` to `remote_path`.
pub fn start_ssh_forward(
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

/// Find the programmer-mcp socket path on the remote host.
pub async fn find_remote_socket(spec: &RemoteSpec, debug_mode: bool) -> anyhow::Result<String> {
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

/// Run a command on the remote host via SSH and return stdout.
pub async fn ssh_command(spec: &RemoteSpec, command: &str) -> anyhow::Result<String> {
    let mut args = spec.ssh_base_args();
    args.push(command.to_string());

    let output = Command::new("ssh").args(&args).output().await?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!("SSH command failed: {stderr}");
    }

    Ok(String::from_utf8_lossy(&output.stdout).to_string())
}

/// Generate a unique session identifier based on the current timestamp.
pub fn generate_session_id() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let ts = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_millis();
    format!("{ts:x}")
}

/// Poll a Unix socket path until it exists, returning `false` on timeout.
pub async fn wait_for_socket(path: &std::path::Path) -> bool {
    for _ in 0..60 {
        if path.exists() {
            return true;
        }
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    }
    false
}
