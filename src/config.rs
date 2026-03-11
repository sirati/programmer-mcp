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

impl LspSpec {
    /// Reconstruct the CLI argument string (e.g. `"rust:rust-analyzer --stdio"`).
    pub fn to_spec_string(&self) -> String {
        let mut parts = vec![self.command.clone()];
        parts.extend(self.args.iter().cloned());
        format!("{}:{}", self.language, parts.join(" "))
    }
}

#[derive(Parser, Debug)]
#[command(name = "programmer-mcp", about = "Multi-LSP MCP server")]
pub struct Config {
    /// Path to workspace directory
    #[arg(long)]
    pub workspace: PathBuf,

    /// LSP servers to connect to (format: language:command [args])
    /// Not required when --debug is active.
    #[arg(long = "lsp")]
    pub lsp_specs: Vec<LspSpec>,

    /// Run in debug/meta mode: exposes rebuild, grab-log, and relay-command tools
    /// instead of the normal LSP tools.
    #[arg(long, default_value_t = false)]
    pub debug: bool,
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

        if !config.debug && config.lsp_specs.is_empty() {
            anyhow::bail!("at least one --lsp spec is required in normal (non-debug) mode");
        }

        Ok(config)
    }
}
