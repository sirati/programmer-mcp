use std::path::{Path, PathBuf};

pub const CONFIG_FILENAME: &str = "debug-mcp.config.toml";

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, Default)]
pub struct DebugConfig {
    #[serde(default)]
    pub lsp: Vec<String>,
}

pub struct ConfigState {
    pub config: DebugConfig,
    pub load_error: Option<String>,
    pub path: PathBuf,
}

impl ConfigState {
    pub fn load(project_root: &Path) -> Self {
        let path = project_root.join(CONFIG_FILENAME);
        let (config, load_error) = load_from_path(&path);
        Self {
            config,
            load_error,
            path,
        }
    }

    pub fn save(&mut self) -> anyhow::Result<()> {
        let content = toml::to_string_pretty(&self.config)
            .map_err(|e| anyhow::anyhow!("failed to serialize config: {e}"))?;
        std::fs::write(&self.path, content)
            .map_err(|e| anyhow::anyhow!("failed to write config file: {e}"))?;
        self.load_error = None;
        Ok(())
    }

    pub fn add_lsp(&mut self, spec: String) -> anyhow::Result<()> {
        if self.config.lsp.iter().any(|s| s == &spec) {
            anyhow::bail!("LSP spec already present: {spec}");
        }
        self.config.lsp.push(spec);
        self.save()
    }

    pub fn remove_lsp(&mut self, language: &str) -> anyhow::Result<()> {
        let before = self.config.lsp.len();
        self.config
            .lsp
            .retain(|s| !s.starts_with(&format!("{language}:")));
        if self.config.lsp.len() == before {
            anyhow::bail!("no LSP spec found for language: {language}");
        }
        self.save()
    }
}

fn load_from_path(path: &Path) -> (DebugConfig, Option<String>) {
    match std::fs::read_to_string(path) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => (DebugConfig::default(), None),
        Err(e) => (
            DebugConfig::default(),
            Some(format!("failed to read {}: {e}", path.display())),
        ),
        Ok(content) => match toml::from_str::<DebugConfig>(&content) {
            Ok(cfg) => (cfg, None),
            Err(e) => (
                DebugConfig::default(),
                Some(format!("invalid config at {}: {e}", path.display())),
            ),
        },
    }
}
