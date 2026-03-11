mod config;
mod lsp;
mod server;
mod tools;
mod watcher;

use std::sync::Arc;

use rmcp::{ServiceExt, transport::stdio};
use tracing_subscriber::EnvFilter;

use config::Config;
use lsp::manager::LspManager;
use server::ProgrammerServer;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env().add_directive(tracing::Level::DEBUG.into()))
        .with_writer(std::io::stderr)
        .with_ansi(false)
        .init();

    tracing::info!("programmer-mcp starting");

    let config = Config::parse_and_validate()?;

    // Change to workspace directory
    std::env::set_current_dir(&config.workspace)?;

    // Start all LSP servers
    let manager = Arc::new(LspManager::start(&config.lsp_specs, &config.workspace).await?);

    // Start file watcher
    let watcher_manager = manager.clone();
    let workspace = config.workspace.clone();
    tokio::spawn(async move {
        watcher::watch_workspace(watcher_manager, &workspace).await;
    });

    // Wait briefly for LSP servers to settle
    tokio::time::sleep(std::time::Duration::from_secs(1)).await;

    // Start MCP server on stdio
    let mcp_server = ProgrammerServer::new(manager.clone());
    let service = mcp_server.serve(stdio()).await.inspect_err(|e| {
        tracing::error!("MCP serve error: {e:?}");
    })?;

    service.waiting().await?;

    // Cleanup
    manager.shutdown().await;
    tracing::info!("programmer-mcp shut down");

    Ok(())
}
