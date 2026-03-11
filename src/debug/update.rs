use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use tokio::sync::Mutex;

use super::build::run_cargo_build;
use super::child::ChildHandle;

pub struct UpdateOutcome {
    pub success: bool,
    pub message: String,
}

impl UpdateOutcome {
    fn ok(message: impl Into<String>) -> Self {
        Self {
            success: true,
            message: message.into(),
        }
    }
    fn err(message: impl Into<String>) -> Self {
        Self {
            success: false,
            message: message.into(),
        }
    }
}

/// Main entry point. Handles both first-time and proxy-mode replacement.
pub async fn run_update_debug_bin(
    project_root: &Path,
    original_args: &[String],
    tested_child: &Arc<Mutex<Option<ChildHandle>>>,
    debug_child: &Arc<Mutex<Option<ChildHandle>>>,
    proxy_mode: &Arc<AtomicBool>,
) -> UpdateOutcome {
    let outcome = run_cargo_build(project_root).await;
    if !outcome.success() {
        return UpdateOutcome::err(format!("Build failed:\n{}", outcome.errors));
    }
    let binary_src = outcome.binary_path.unwrap();

    if let Err(reason) = test_debug_start(&binary_src, original_args, project_root).await {
        return UpdateOutcome::err(format!(
            "Build succeeded but the new binary {reason} before becoming ready in --debug mode.",
        ));
    }

    match replace_self_and_launch(&binary_src, original_args, project_root).await {
        Err(e) => UpdateOutcome::err(format!("Failed to replace binary: {e}")),
        Ok(new_binary_path) => {
            finish_update(
                &new_binary_path,
                original_args,
                project_root,
                tested_child,
                debug_child,
                proxy_mode,
            )
            .await
        }
    }
}

/// Spawn binary with --debug args, wait for it to be ready, then kill it.
/// Returns `Ok(())` if it survived the ready check, or `Err(reason)` with a description.
async fn test_debug_start(
    binary: &Path,
    original_args: &[String],
    workspace: &Path,
) -> Result<(), String> {
    let child = ChildHandle::spawn(binary, original_args, workspace)
        .await
        .map_err(|e| format!("failed to spawn: {e}"))?;
    match child.wait_for_ready().await {
        Ok(()) => {
            child.kill().await;
            Ok(())
        }
        Err(exit_info) => Err(exit_info.describe()),
    }
}

/// Safely replace the current binary with a new build artifact.
///
/// Strategy: move old binary aside, copy new one in, verify, then delete old.
/// On failure, move the old binary back so we never end up with nothing.
fn replace_self_binary(binary_src: &Path) -> anyhow::Result<PathBuf> {
    let self_path = std::env::current_exe()
        .map_err(|e| anyhow::anyhow!("cannot determine own executable path: {e}"))?;
    let backup_path = self_path.with_extension("old");

    // Step 1: move current binary aside
    if self_path.exists() {
        std::fs::rename(&self_path, &backup_path).map_err(|e| {
            anyhow::anyhow!(
                "failed to move old binary to {}: {e}",
                backup_path.display()
            )
        })?;
    }

    // Step 2: copy new binary in
    let copy_result = std::fs::copy(binary_src, &self_path);

    // Step 3: verify the new binary exists
    if copy_result.is_err() || !self_path.exists() {
        // Restore old binary
        if backup_path.exists() {
            let _ = std::fs::rename(&backup_path, &self_path);
        }
        let err = copy_result.err().map(|e| e.to_string()).unwrap_or_default();
        anyhow::bail!(
            "failed to copy new binary to {}: {err}",
            self_path.display()
        );
    }

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = std::fs::metadata(&self_path)?.permissions();
        perms.set_mode(perms.mode() | 0o111);
        std::fs::set_permissions(&self_path, perms)?;
    }

    // Step 4: delete the backup
    let _ = std::fs::remove_file(&backup_path);

    Ok(self_path)
}

async fn replace_self_and_launch(
    binary_src: &Path,
    _original_args: &[String],
    _workspace: &Path,
) -> anyhow::Result<PathBuf> {
    replace_self_binary(binary_src)
}

/// Start the new debug child from `new_binary_path`, then swap out old state.
async fn finish_update(
    new_binary_path: &Path,
    original_args: &[String],
    workspace: &Path,
    tested_child: &Arc<Mutex<Option<ChildHandle>>>,
    debug_child: &Arc<Mutex<Option<ChildHandle>>>,
    proxy_mode: &Arc<AtomicBool>,
) -> UpdateOutcome {
    let new_debug = match ChildHandle::spawn(new_binary_path, original_args, workspace).await {
        Err(e) => {
            return UpdateOutcome::err(format!(
                "Replaced binary but failed to spawn new debug process: {e}"
            ))
        }
        Ok(c) => c,
    };

    if let Err(exit_info) = new_debug.wait_for_ready().await {
        let logs = new_debug.search_logs(None, 30).await;
        new_debug.kill().await;
        return UpdateOutcome::err(format!(
            "Replaced binary but new debug process {} before becoming ready.\n--- stderr ---\n{}",
            exit_info.describe(),
            if logs.is_empty() {
                "(no output)".to_string()
            } else {
                logs.join("\n")
            },
        ));
    }

    if let Err(e) = new_debug.ensure_mcp_initialized().await {
        let logs = new_debug.search_logs(None, 30).await;
        new_debug.kill().await;
        return UpdateOutcome::err(format!(
            "New debug process started but MCP initialization failed: {e}\n--- stderr ---\n{}",
            if logs.is_empty() {
                "(no output)".to_string()
            } else {
                logs.join("\n")
            },
        ));
    }

    let already_proxying = proxy_mode.load(Ordering::Relaxed);

    if already_proxying {
        // Replace the existing debug child.
        let mut dc = debug_child.lock().await;
        if let Some(old) = dc.take() {
            old.kill().await;
        }
        *dc = Some(new_debug);
        UpdateOutcome::ok("Debug binary updated and debug child replaced.")
    } else {
        // First time: stop the tested child, enter proxy mode.
        let mut tc = tested_child.lock().await;
        if let Some(old) = tc.take() {
            old.kill().await;
        }
        drop(tc);

        proxy_mode.store(true, Ordering::Relaxed);

        let mut dc = debug_child.lock().await;
        *dc = Some(new_debug);

        UpdateOutcome::ok(
            "Debug binary updated. Tested child stopped. All further traffic is now \
             forwarded to the new debug process (except `update_debug_bin`).",
        )
    }
}
