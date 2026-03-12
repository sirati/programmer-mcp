mod background;
mod config;
mod debug;
mod ipc;
mod lsp;
mod nix;
mod relay;
mod remote;
mod server;
mod tools;
mod watcher;

use std::sync::Arc;

use rmcp::{transport::stdio, ServiceExt};
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::EnvFilter;

use background::BackgroundManager;
use config::Config;
use debug::server::DebugServer;
use lsp::manager::LspManager;
use server::ProgrammerServer;
use tools::diagnostics_cache::DiagnosticsCache;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Check for `-- message` mode: send message to running instance and exit
    let raw_args: Vec<String> = std::env::args().collect();
    if let Some(sep) = raw_args.iter().position(|a| a == "--") {
        let message = raw_args[sep + 1..].join(" ");
        if message.is_empty() {
            anyhow::bail!("no message provided after --");
        }
        // We need --workspace to know the socket path
        let workspace = raw_args
            .iter()
            .position(|a| a == "--workspace")
            .and_then(|i| raw_args.get(i + 1))
            .map(std::path::PathBuf::from)
            .ok_or_else(|| anyhow::anyhow!("--workspace is required when sending messages"))?;
        let workspace = workspace.canonicalize()?;
        ipc::send_message(&workspace, &message).await?;
        return Ok(());
    }

    let config = Config::parse_and_validate()?;

    // Set up tracing.  Remote-proxy mode is launched as a silent subprocess by MCP clients
    // (Zed, Claude, …) whose stderr is never shown.  Write a fresh timestamped log file so
    // failures are always diagnosable.
    if config.remote.is_some() {
        let log_dir = Config::socket_dir().join("logs");
        std::fs::create_dir_all(&log_dir)?;

        // One file per invocation, named by unix timestamp in millis.
        let ts = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis();
        let remote_str = config
            .remote
            .as_deref()
            .unwrap_or("unknown")
            .replace('/', "_");
        let log_path = log_dir.join(format!("remote-{remote_str}-{ts}.log"));

        let log_file = std::fs::OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(true)
            .open(&log_path)?;

        let file_layer = tracing_subscriber::fmt::layer()
            .with_writer(std::sync::Mutex::new(log_file))
            .with_ansi(false);
        let stderr_layer = tracing_subscriber::fmt::layer()
            .with_writer(std::io::stderr)
            .with_ansi(false);

        tracing_subscriber::registry()
            .with(EnvFilter::from_default_env().add_directive(tracing::Level::DEBUG.into()))
            .with(file_layer)
            .with(stderr_layer)
            .init();

        tracing::info!(
            "programmer-mcp remote proxy: logging to {}",
            log_path.display()
        );
        return remote::run_remote_client(config).await;
    }

    // Normal and debug modes just log to stderr.
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env().add_directive(tracing::Level::DEBUG.into()))
        .with_writer(std::io::stderr)
        .with_ansi(false)
        .init();

    if config.debug {
        run_debug_server(config).await
    } else {
        run_normal_server(config).await
    }
}

async fn run_debug_server(config: Config) -> anyhow::Result<()> {
    tracing::info!("programmer-mcp starting in debug mode");

    let original_args: Vec<String> = std::env::args().skip(1).collect();
    let cli_lsp_specs = config
        .lsp_specs
        .iter()
        .map(|s| s.to_spec_string())
        .collect();
    let server = DebugServer::new(
        config.workspace().to_path_buf(),
        cli_lsp_specs,
        original_args,
    );

    // Auto-rebuild on startup so the child is ready without a manual `rebuild` call.
    let rebuild_server = server.clone();
    tokio::spawn(async move {
        match rebuild_server.run_rebuild().await {
            Ok(msg) => tracing::info!("auto-rebuild: {msg}"),
            Err(e) => tracing::warn!("auto-rebuild skipped or failed: {e}"),
        }
    });

    // Start remote listener for debug server too
    let mut remote_listener = remote::RemoteListener::new(config.socket_path());
    remote_listener.start(server.clone());

    let service = server.serve(stdio()).await.inspect_err(|e| {
        tracing::error!("debug MCP serve error: {e:?}");
    })?;

    service.waiting().await?;
    remote_listener.shutdown();
    tracing::info!("programmer-mcp debug server shut down");
    Ok(())
}

async fn run_normal_server(config: Config) -> anyhow::Result<()> {
    tracing::info!("programmer-mcp starting");

    let workspace = config.workspace().to_path_buf();
    std::env::set_current_dir(&workspace)?;

    let manager = Arc::new(LspManager::start(&config.lsp_specs, &workspace).await?);
    let message_bus = ipc::HumanMessageBus::start(&workspace);
    let background = BackgroundManager::new(&workspace);
    let diag_cache = DiagnosticsCache::new(&workspace);

    let watcher_manager = manager.clone();
    let watcher_cache = diag_cache.clone();
    let watcher_workspace = workspace.clone();
    tokio::spawn(async move {
        watcher::watch_workspace(watcher_manager, watcher_cache, &watcher_workspace).await;
    });

    tokio::time::sleep(std::time::Duration::from_secs(1)).await;

    let mcp_server = ProgrammerServer::new(
        manager.clone(),
        message_bus,
        background,
        workspace.clone(),
        diag_cache,
        config.allow_file_edit,
        config.length_limits(),
    );

    // Start remote listener for SSH-based sessions
    let mut remote_listener = remote::RemoteListener::new(config.socket_path());
    remote_listener.start(mcp_server.clone());
    let service = mcp_server.serve(stdio()).await.inspect_err(|e| {
        tracing::error!("MCP serve error: {e:?}");
    })?;

    service.waiting().await?;

    remote_listener.shutdown();
    manager.shutdown().await;
    tracing::info!("programmer-mcp shut down");
    Ok(())
}
