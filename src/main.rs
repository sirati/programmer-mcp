mod background;
mod config;
mod debug;
mod ipc;
mod lsp;
mod relay;
mod remote;
mod server;
mod tools;
mod watcher;

use std::sync::Arc;

use rmcp::{transport::stdio, ServiceExt};
use tracing_subscriber::EnvFilter;

use background::BackgroundManager;
use config::Config;
use debug::server::DebugServer;
use lsp::manager::LspManager;
use server::ProgrammerServer;

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

    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env().add_directive(tracing::Level::DEBUG.into()))
        .with_writer(std::io::stderr)
        .with_ansi(false)
        .init();

    let config = Config::parse_and_validate()?;

    if config.remote.is_some() {
        remote::run_remote_client(config).await
    } else if config.debug {
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
    let server = DebugServer::new(config.workspace().to_path_buf(), cli_lsp_specs, original_args);

    // Start remote listener for debug server too
    let remote_listener = remote::RemoteListener::new(config.socket_path());
    remote_listener.start(server.clone());

    let service = server.serve(stdio()).await.inspect_err(|e| {
        tracing::error!("debug MCP serve error: {e:?}");
    })?;

    service.waiting().await?;
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

    let watcher_manager = manager.clone();
    let watcher_workspace = workspace.clone();
    tokio::spawn(async move {
        watcher::watch_workspace(watcher_manager, &watcher_workspace).await;
    });

    tokio::time::sleep(std::time::Duration::from_secs(1)).await;

    let mcp_server = ProgrammerServer::new(manager.clone(), message_bus, background);

    // Start remote listener for SSH-based sessions
    let remote_listener = remote::RemoteListener::new(config.socket_path());
    remote_listener.start(mcp_server.clone());
    let service = mcp_server.serve(stdio()).await.inspect_err(|e| {
        tracing::error!("MCP serve error: {e:?}");
    })?;

    service.waiting().await?;

    manager.shutdown().await;
    tracing::info!("programmer-mcp shut down");
    Ok(())
}
