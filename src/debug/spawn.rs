//! Helpers for spawning and replacing child processes.

use std::path::Path;

use tokio::sync::Mutex;

use super::child::ChildHandle;

/// Copy a binary to a temp directory, then spawn it, wait for ready,
/// and atomically replace the current child.
pub async fn replace_child(
    child_mutex: &Mutex<Option<ChildHandle>>,
    binary_src: &Path,
    args: &[String],
    workspace: &Path,
) -> anyhow::Result<String> {
    let tmp_binary = copy_to_tmp(binary_src)?;
    let new_child = ChildHandle::spawn(&tmp_binary, args, workspace).await?;

    if let Err(exit_info) = new_child.wait_for_ready().await {
        let logs = new_child.search_logs(None, 30).await;
        new_child.kill().await;
        let log_snippet = if logs.is_empty() {
            "(no stderr output captured)".to_string()
        } else {
            logs.join("\n")
        };
        anyhow::bail!(
            "new child {} before becoming ready.\n\
             workspace: {}\n\
             args: {}\n\
             --- child stderr ---\n{log_snippet}",
            exit_info.describe(),
            workspace.display(),
            args.join(" "),
        );
    }

    let mut guard = child_mutex.lock().await;
    let had_previous = guard.is_some();
    if let Some(old) = guard.take() {
        old.kill().await;
    }
    *guard = Some(new_child);

    Ok(if had_previous {
        "Rebuilt and restarted.".to_string()
    } else {
        "Built and started.".to_string()
    })
}

/// Copy a binary to a unique temp directory and make it executable.
pub fn copy_to_tmp(src: &Path) -> anyhow::Result<std::path::PathBuf> {
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();
    let tmp_dir = std::env::temp_dir().join(format!("programmer-mcp-debug-{ts}"));
    std::fs::create_dir_all(&tmp_dir)?;

    let name = src
        .file_name()
        .ok_or_else(|| anyhow::anyhow!("binary has no filename"))?;
    let dest = tmp_dir.join(name);
    std::fs::copy(src, &dest)?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = std::fs::metadata(&dest)?.permissions();
        perms.set_mode(perms.mode() | 0o111);
        std::fs::set_permissions(&dest, perms)?;
    }

    Ok(dest)
}
