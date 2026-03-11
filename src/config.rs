use std::path::PathBuf;

use clap::Parser;

/// An LSP specification in the format `language:command [args...]`
#[derive(Debug, Clone)]
pub struct LspSpec {
    pub language: String,
    pub command: String,
    pub args: Vec<String>,
}

impl std::str::FromStr for LspSpec {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let (language, rest) = s
            .split_once(':')
            .ok_or_else(|| format!("invalid LSP spec '{s}', expected 'language:command [args]'"))?;

        let mut parts = rest.split_whitespace();
        let command = parts
            .next()
            .ok_or_else(|| format!("missing command in LSP spec '{s}'"))?
            .to_string();
        let args = parts.map(String::from).collect();

        Ok(Self {
            language: language.to_string(),
            command,
            args,
        })
    }
}

#[derive(Parser, Debug)]
#[command(name = "programmer-mcp", about = "Multi-LSP MCP server")]
pub struct Config {
    /// Path to workspace directory
    #[arg(long)]
    pub workspace: PathBuf,

    /// LSP servers to connect to (format: language:command [args])
    #[arg(long = "lsp", required = true)]
    pub lsp_specs: Vec<LspSpec>,
}

impl Config {
    pub fn parse_and_validate() -> anyhow::Result<Self> {
        let mut config = Self::parse();

        config.workspace = config.workspace.canonicalize().map_err(|e| {
            anyhow::anyhow!(
                "workspace directory '{}' not accessible: {e}",
                config.workspace.display()
            )
        })?;

        if !config.workspace.is_dir() {
            anyhow::bail!(
                "workspace '{}' is not a directory",
                config.workspace.display()
            );
        }

        Ok(config)
    }
}
