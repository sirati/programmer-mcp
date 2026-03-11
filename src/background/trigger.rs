use std::collections::HashMap;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use tokio::sync::watch;
use tracing::debug;

/// Persistent trigger configuration saved to disk.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TriggerConfig {
    pub name: String,
    /// Substring pattern to match in process output.
    pub pattern: String,
    /// Lines of context to capture before the trigger line.
    #[serde(default)]
    pub lines_before: usize,
    /// Lines of context to capture after the trigger line.
    #[serde(default = "default_lines_after")]
    pub lines_after: usize,
    /// Timeout in ms for collecting after-lines.
    #[serde(default = "default_timeout")]
    pub timeout_ms: u64,
    /// If set, auto-attach to processes started with this group.
    pub group: Option<String>,
}

fn default_lines_after() -> usize {
    5
}

fn default_timeout() -> u64 {
    3000
}

/// Result of a trigger firing.
#[derive(Debug, Clone)]
pub struct TriggerResult {
    pub trigger_name: String,
    pub matched_line: String,
    pub context: Vec<String>,
}

impl std::fmt::Display for TriggerResult {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "--- Trigger '{}' ---\n", self.trigger_name)?;
        for line in &self.context {
            writeln!(f, "{line}")?;
        }
        Ok(())
    }
}

/// Manages trigger definitions and their state.
pub struct TriggerManager {
    trigger_dir: PathBuf,
    configs: HashMap<String, TriggerConfig>,
    /// Pending results from triggers that fired but weren't awaited.
    pub pending_results: Vec<TriggerResult>,
    /// Notifies when a trigger fires.
    fire_tx: watch::Sender<()>,
    fire_rx: watch::Receiver<()>,
}

impl TriggerManager {
    pub fn new(trigger_dir: PathBuf) -> Self {
        let (fire_tx, fire_rx) = watch::channel(());
        let mut mgr = Self {
            trigger_dir,
            configs: HashMap::new(),
            pending_results: Vec::new(),
            fire_tx,
            fire_rx,
        };
        mgr.load_all();
        mgr
    }

    /// Load all trigger configs from disk.
    fn load_all(&mut self) {
        if !self.trigger_dir.exists() {
            return;
        }
        let Ok(entries) = std::fs::read_dir(&self.trigger_dir) else {
            return;
        };
        for entry in entries.flatten() {
            if entry.path().extension().and_then(|e| e.to_str()) == Some("json") {
                if let Ok(content) = std::fs::read_to_string(entry.path()) {
                    if let Ok(config) = serde_json::from_str::<TriggerConfig>(&content) {
                        debug!(trigger = %config.name, "loaded trigger config");
                        self.configs.insert(config.name.clone(), config);
                    }
                }
            }
        }
    }

    /// Define or update a trigger, saving to disk.
    pub fn define(&mut self, config: TriggerConfig) -> Result<(), String> {
        std::fs::create_dir_all(&self.trigger_dir)
            .map_err(|e| format!("cannot create trigger dir: {e}"))?;
        let path = self.trigger_dir.join(format!("{}.json", config.name));
        let json =
            serde_json::to_string_pretty(&config).map_err(|e| format!("serialize error: {e}"))?;
        std::fs::write(&path, json).map_err(|e| format!("write error: {e}"))?;
        self.configs.insert(config.name.clone(), config);
        Ok(())
    }

    /// Get triggers that should auto-attach to a process group.
    pub fn triggers_for_group(&self, group: &str) -> Vec<&TriggerConfig> {
        self.configs
            .values()
            .filter(|c| c.group.as_deref() == Some(group))
            .collect()
    }

    /// Get a trigger config by name.
    pub fn get(&self, name: &str) -> Option<&TriggerConfig> {
        self.configs.get(name)
    }

    /// Record that a trigger fired.
    pub fn record_fire(&mut self, result: TriggerResult) {
        debug!(trigger = %result.trigger_name, "trigger fired");
        self.pending_results.push(result);
        let _ = self.fire_tx.send(());
    }

    /// Take all pending results.
    pub fn take_pending(&mut self) -> Vec<TriggerResult> {
        std::mem::take(&mut self.pending_results)
    }

    /// Check if any trigger with the given name has fired.
    pub fn has_fired(&self, name: &str) -> bool {
        self.pending_results.iter().any(|r| r.trigger_name == name)
    }

    /// Get a receiver for trigger fire notifications.
    pub fn subscribe(&self) -> watch::Receiver<()> {
        self.fire_rx.clone()
    }

    /// List all defined triggers.
    pub fn list(&self) -> Vec<&TriggerConfig> {
        self.configs.values().collect()
    }
}
