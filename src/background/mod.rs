pub mod process;
pub mod trigger;

use std::path::{Path, PathBuf};
use std::sync::Arc;

use tokio::sync::Mutex;

use process::ProcessManager;
use trigger::TriggerManager;

/// Manages background processes and their triggers.
pub struct BackgroundManager {
    pub processes: Arc<Mutex<ProcessManager>>,
    pub triggers: Arc<Mutex<TriggerManager>>,
}

impl BackgroundManager {
    pub fn new(workspace: &Path) -> Arc<Self> {
        let trigger_dir = workspace.join(".programmer-mcp").join("triggers");
        Arc::new(Self {
            processes: Arc::new(Mutex::new(ProcessManager::new())),
            triggers: Arc::new(Mutex::new(TriggerManager::new(trigger_dir))),
        })
    }
}
