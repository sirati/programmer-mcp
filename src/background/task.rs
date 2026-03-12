//! Named task management with persistent JSON storage.
//!
//! Tasks are saved as `.programmer-mcp/tasks/{name}.json` in the workspace.
//! Each task may have named subtasks, all stored in the same file.

use std::collections::BTreeMap;
use std::fmt::Write;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use tracing::{debug, warn};

// ── data types ────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Subtask {
    pub name: String,
    pub description: String,
    pub completed: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Task {
    pub name: String,
    pub description: String,
    pub completed: bool,
    pub subtasks: Vec<Subtask>,
}

impl Task {
    fn new(name: impl Into<String>, description: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            description: description.into(),
            completed: false,
            subtasks: Vec::new(),
        }
    }

    /// One-line summary used for list output.
    fn summary(&self) -> String {
        let done_mark = if self.completed { "[x]" } else { "[ ]" };
        let total = self.subtasks.len();
        let done = self.subtasks.iter().filter(|s| s.completed).count();
        if total > 0 {
            format!(
                "{done_mark} {} ({done}/{total} subtasks)  – {}",
                self.name,
                first_line(&self.description)
            )
        } else {
            format!(
                "{done_mark} {}  – {}",
                self.name,
                first_line(&self.description)
            )
        }
    }
}

fn first_line(s: &str) -> &str {
    s.lines().next().unwrap_or(s)
}

// ── task manager ─────────────────────────────────────────────────────────────

pub struct TaskManager {
    task_dir: PathBuf,
    /// In-memory cache: task name → task.
    cache: BTreeMap<String, Task>,
}

impl TaskManager {
    /// Create a new manager and pre-load all tasks from disk.
    pub fn new(task_dir: PathBuf) -> Self {
        let mut mgr = Self {
            task_dir,
            cache: BTreeMap::new(),
        };
        mgr.load_all();
        mgr
    }

    // ── public API ────────────────────────────────────────────────────────────

    /// Create or replace a task.
    pub fn set_task(&mut self, name: &str, description: &str) -> Result<(), String> {
        let task = Task::new(name, description);
        self.save(&task)?;
        self.cache.insert(name.to_string(), task);
        Ok(())
    }

    /// Update an existing task's description (replaces or appends) and/or
    /// mark it completed/uncompleted.
    pub fn update_task(
        &mut self,
        name: &str,
        new_description: Option<&str>,
        append_description: Option<&str>,
        completed: Option<bool>,
    ) -> Result<(), String> {
        let task = self
            .cache
            .get_mut(name)
            .ok_or_else(|| format!("task '{name}' not found"))?;

        if let Some(desc) = new_description {
            task.description = desc.to_string();
        }
        if let Some(append) = append_description {
            if task.description.is_empty() {
                task.description = append.to_string();
            } else {
                task.description.push('\n');
                task.description.push_str(append);
            }
        }
        if let Some(done) = completed {
            task.completed = done;
        }

        let task = task.clone();
        self.save(&task)
    }

    /// Add (or replace) a subtask on a task.
    pub fn add_subtask(
        &mut self,
        task_name: &str,
        subtask_name: &str,
        description: &str,
    ) -> Result<(), String> {
        let task = self
            .cache
            .get_mut(task_name)
            .ok_or_else(|| format!("task '{task_name}' not found"))?;

        if let Some(existing) = task.subtasks.iter_mut().find(|s| s.name == subtask_name) {
            existing.description = description.to_string();
        } else {
            task.subtasks.push(Subtask {
                name: subtask_name.to_string(),
                description: description.to_string(),
                completed: false,
            });
        }

        let task = task.clone();
        self.save(&task)
    }

    /// Mark a task as completed.
    pub fn complete_task(&mut self, name: &str) -> Result<(), String> {
        self.update_task(name, None, None, Some(true))
    }

    /// Mark a subtask of a task as completed.
    pub fn complete_subtask(&mut self, task_name: &str, subtask_name: &str) -> Result<(), String> {
        let task = self
            .cache
            .get_mut(task_name)
            .ok_or_else(|| format!("task '{task_name}' not found"))?;

        let sub = task
            .subtasks
            .iter_mut()
            .find(|s| s.name == subtask_name)
            .ok_or_else(|| format!("subtask '{subtask_name}' not found in task '{task_name}'"))?;

        sub.completed = true;

        let task = task.clone();
        self.save(&task)
    }

    /// Return a formatted list of tasks.
    /// `include_completed` – if false (default) only pending tasks are listed.
    pub fn list_tasks(&self, include_completed: bool) -> String {
        let tasks: Vec<&Task> = self
            .cache
            .values()
            .filter(|t| include_completed || !t.completed)
            .collect();

        if tasks.is_empty() {
            return if include_completed {
                "no tasks".to_string()
            } else {
                "no pending tasks".to_string()
            };
        }

        let mut out = String::new();
        for task in tasks {
            let _ = writeln!(out, "{}", task.summary());
        }
        out.trim_end().to_string()
    }

    /// Return a formatted list of subtasks for a task.
    /// `include_completed` – if false (default) only pending subtasks are listed.
    pub fn list_subtasks(&self, task_name: &str, include_completed: bool) -> String {
        let task = match self.cache.get(task_name) {
            Some(t) => t,
            None => return format!("task '{task_name}' not found"),
        };

        let subs: Vec<&Subtask> = task
            .subtasks
            .iter()
            .filter(|s| include_completed || !s.completed)
            .collect();

        if subs.is_empty() {
            return if include_completed {
                format!("no subtasks in '{task_name}'")
            } else {
                format!("no pending subtasks in '{task_name}'")
            };
        }

        let mut out = String::new();
        for sub in subs {
            let done_mark = if sub.completed { "[x]" } else { "[ ]" };
            let _ = writeln!(
                out,
                "{done_mark} {}  – {}",
                sub.name,
                first_line(&sub.description)
            );
        }
        out.trim_end().to_string()
    }

    // ── persistence ───────────────────────────────────────────────────────────

    fn task_path(&self, name: &str) -> PathBuf {
        // Sanitize: replace path separators to keep it as a single filename.
        let safe_name = name.replace(['/', '\\', ':'], "_");
        self.task_dir.join(format!("{safe_name}.json"))
    }

    fn save(&self, task: &Task) -> Result<(), String> {
        if let Err(e) = std::fs::create_dir_all(&self.task_dir) {
            return Err(format!("cannot create task dir: {e}"));
        }
        let path = self.task_path(&task.name);
        let json = serde_json::to_string_pretty(task)
            .map_err(|e| format!("cannot serialize task: {e}"))?;
        std::fs::write(&path, json).map_err(|e| format!("cannot write task file: {e}"))
    }

    /// Load all task JSON files from `task_dir` at startup.
    fn load_all(&mut self) {
        let dir = match std::fs::read_dir(&self.task_dir) {
            Ok(d) => d,
            Err(_) => return, // directory doesn't exist yet — that's fine
        };
        for entry in dir.flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("json") {
                continue;
            }
            match load_task_file(&path) {
                Ok(task) => {
                    debug!(name = %task.name, "loaded task from disk");
                    self.cache.insert(task.name.clone(), task);
                }
                Err(e) => {
                    warn!(path = %path.display(), "failed to load task file: {e}");
                }
            }
        }
    }
}

fn load_task_file(path: &Path) -> Result<Task, String> {
    let content = std::fs::read_to_string(path).map_err(|e| format!("read error: {e}"))?;
    serde_json::from_str(&content).map_err(|e| format!("parse error: {e}"))
}
