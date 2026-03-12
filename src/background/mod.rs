pub mod process;
pub mod task;
pub mod trigger;

use std::path::Path;
use std::sync::Arc;

use tokio::sync::Mutex;

use process::ProcessManager;
use task::TaskManager;
use trigger::TriggerManager;

/// Manages background processes, triggers, and tasks.
pub struct BackgroundManager {
    pub processes: Arc<Mutex<ProcessManager>>,
    pub triggers: Arc<Mutex<TriggerManager>>,
    pub tasks: Arc<Mutex<TaskManager>>,
}

impl BackgroundManager {
    pub fn new(workspace: &Path) -> Arc<Self> {
        let base_dir = workspace.join(".programmer-mcp");
        let trigger_dir = base_dir.join("triggers");
        let task_dir = base_dir.join("tasks");
        Arc::new(Self {
            processes: Arc::new(Mutex::new(ProcessManager::new())),
            triggers: Arc::new(Mutex::new(TriggerManager::new(trigger_dir))),
            tasks: Arc::new(Mutex::new(TaskManager::new(task_dir))),
        })
    }
}
