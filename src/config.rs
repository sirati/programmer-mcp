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
    /// Path to workspace directory (not required when --remote is used)
    #[arg(long)]
    pub workspace: Option<PathBuf>,

    /// LSP servers to connect to (format: language:command [args])
    /// Not required when --debug is active.
    #[arg(long = "lsp")]
    pub lsp_specs: Vec<LspSpec>,

    /// Run in debug/meta mode: exposes rebuild, grab-log, and relay-command tools
    /// instead of the normal LSP tools.
    #[arg(long, default_value_t = false)]
    pub debug: bool,

    /// Connect to a remote programmer-mcp instance via SSH.
    /// Format: [user@]host[:port]
    #[arg(long)]
    pub remote: Option<String>,

    /// Enable the `edit file` command for raw file editing.
    /// Disabled by default for safety.
    #[arg(long, default_value_t = false)]
    pub allow_file_edit: bool,
}

impl Config {
    pub fn parse_and_validate() -> anyhow::Result<Self> {
        let mut config = Self::parse();

        // Remote mode doesn't need workspace or LSP specs
        if config.remote.is_some() {
            return Ok(config);
        }

        let workspace = config
            .workspace
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("--workspace is required (unless using --remote)"))?;

        let canonical = workspace.canonicalize().map_err(|e| {
            anyhow::anyhow!(
                "workspace directory '{}' not accessible: {e}",
                workspace.display()
            )
        })?;

        if !canonical.is_dir() {
            anyhow::bail!("workspace '{}' is not a directory", canonical.display());
        }

        config.workspace = Some(canonical);

        if !config.debug && config.lsp_specs.is_empty() {
            anyhow::bail!("at least one --lsp spec is required in normal (non-debug) mode");
        }

        Ok(config)
    }

    /// Get the validated workspace path. Panics if called without workspace.
    pub fn workspace(&self) -> &std::path::Path {
        self.workspace.as_deref().expect("workspace must be set")
    }

    /// Compute the socket directory for remote access: ~/.local/share/programmer-mcp/
    pub fn socket_dir() -> PathBuf {
        let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".to_string());
        PathBuf::from(home).join(".local/share/programmer-mcp")
    }

    /// Compute the socket name for this instance.
    /// Debug mode → "debug-mcp.sock"
    /// Normal mode → workspace path with / replaced by _ plus ".sock"
    pub fn socket_name(&self) -> String {
        if self.debug {
            "debug-mcp.sock".to_string()
        } else {
            let ws = self.workspace();
            let encoded = ws
                .to_string_lossy()
                .trim_start_matches('/')
                .replace('/', "_");
            format!("{encoded}.sock")
        }
    }

    /// Full path to the control socket for this instance.
    pub fn socket_path(&self) -> PathBuf {
        Self::socket_dir().join(self.socket_name())
    }
}
