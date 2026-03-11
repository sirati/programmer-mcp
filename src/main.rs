mod config;
mod debug;
mod lsp;
mod server;
mod tools;
mod watcher;

use std::sync::Arc;

use rmcp::{transport::stdio, ServiceExt};
use tracing_subscriber::EnvFilter;

use config::Config;
use debug::server::DebugServer;
use lsp::manager::LspManager;
use server::ProgrammerServer;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env().add_directive(tracing::Level::DEBUG.into()))
        .with_writer(std::io::stderr)
        .with_ansi(false)
        .init();

    let config = Config::parse_and_validate()?;

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
    let server = DebugServer::new(config.workspace, cli_lsp_specs, original_args);

    let service = server.serve(stdio()).await.inspect_err(|e| {
        tracing::error!("debug MCP serve error: {e:?}");
    })?;

    service.waiting().await?;
    tracing::info!("programmer-mcp debug server shut down");
    Ok(())
}

async fn run_normal_server(config: Config) -> anyhow::Result<()> {
    tracing::info!("programmer-mcp starting");

    std::env::set_current_dir(&config.workspace)?;

    let manager = Arc::new(LspManager::start(&config.lsp_specs, &config.workspace).await?);

    let watcher_manager = manager.clone();
    let workspace = config.workspace.clone();
    tokio::spawn(async move {
        watcher::watch_workspace(watcher_manager, &workspace).await;
    });

    tokio::time::sleep(std::time::Duration::from_secs(1)).await;

    let mcp_server = ProgrammerServer::new(manager.clone());
    let service = mcp_server.serve(stdio()).await.inspect_err(|e| {
        tracing::error!("MCP serve error: {e:?}");
    })?;

    service.waiting().await?;

    manager.shutdown().await;
    tracing::info!("programmer-mcp shut down");
    Ok(())
}
