use std::collections::HashMap;
use std::sync::{Arc, Mutex as StdMutex};

use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::{Child, Command};
use tokio::sync::watch;
use tracing::debug;

/// Shared output buffer written by background tasks, read by the manager.
type OutputBuf = Arc<StdMutex<Vec<String>>>;

/// A running background process.
pub struct BackgroundProcess {
    pub name: String,
    pub group: Option<String>,
    output: OutputBuf,
    new_line_tx: watch::Sender<()>,
    _child: Child,
}

impl BackgroundProcess {
    /// Search output lines matching a substring.
    pub fn search(&self, pattern: &str) -> Vec<String> {
        let buf = self.output.lock().unwrap();
        buf.iter()
            .filter(|l| l.contains(pattern))
            .cloned()
            .collect()
    }

    /// Get the last N output lines.
    #[allow(dead_code)] // TODO: expose via DSL
    pub fn tail(&self, n: usize) -> Vec<String> {
        let buf = self.output.lock().unwrap();
        buf.iter().rev().take(n).rev().cloned().collect()
    }

    /// Get a receiver that notifies on new lines.
    pub fn subscribe(&self) -> watch::Receiver<()> {
        self.new_line_tx.subscribe()
    }

    /// Get current line count.
    pub fn line_count(&self) -> usize {
        self.output.lock().unwrap().len()
    }

    /// Get lines from a starting index.
    pub fn lines_from(&self, start: usize) -> Vec<String> {
        let buf = self.output.lock().unwrap();
        buf.iter().skip(start).cloned().collect()
    }
}

/// Manages all background processes.
pub struct ProcessManager {
    processes: HashMap<String, BackgroundProcess>,
}

impl ProcessManager {
    pub fn new() -> Self {
        Self {
            processes: HashMap::new(),
        }
    }

    /// Start a new background process.
    pub fn start(
        &mut self,
        name: String,
        group: Option<String>,
        command: &str,
        args: &[String],
    ) -> Result<(), String> {
        if self.processes.contains_key(&name) {
            return Err(format!("process '{name}' already running"));
        }

        let mut child = Command::new(command)
            .args(args)
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .kill_on_drop(true)
            .spawn()
            .map_err(|e| format!("failed to start '{command}': {e}"))?;

        let (tx, _rx) = watch::channel(());
        let output: OutputBuf = Arc::new(StdMutex::new(Vec::new()));

        // Spawn stdout reader
        if let Some(stdout) = child.stdout.take() {
            let buf = output.clone();
            let tx = tx.clone();
            let pname = name.clone();
            tokio::spawn(async move {
                let mut lines = BufReader::new(stdout).lines();
                while let Ok(Some(line)) = lines.next_line().await {
                    debug!(process = %pname, "stdout: {line}");
                    buf.lock().unwrap().push(line);
                    let _ = tx.send(());
                }
            });
        }

        // Spawn stderr reader
        if let Some(stderr) = child.stderr.take() {
            let buf = output.clone();
            let tx = tx.clone();
            let pname = name.clone();
            tokio::spawn(async move {
                let mut lines = BufReader::new(stderr).lines();
                while let Ok(Some(line)) = lines.next_line().await {
                    debug!(process = %pname, "stderr: {line}");
                    buf.lock().unwrap().push(line);
                    let _ = tx.send(());
                }
            });
        }

        self.processes.insert(
            name.clone(),
            BackgroundProcess {
                name,
                group,
                output,
                new_line_tx: tx,
                _child: child,
            },
        );

        Ok(())
    }

    /// Get a process by name.
    pub fn get(&self, name: &str) -> Option<&BackgroundProcess> {
        self.processes.get(name)
    }

    /// Search across processes by name and/or group.
    pub fn search_output(
        &self,
        name: Option<&str>,
        group: Option<&str>,
        pattern: &str,
    ) -> Vec<(String, Vec<String>)> {
        let mut results = Vec::new();
        for proc in self.processes.values() {
            if let Some(n) = name {
                if proc.name != n {
                    continue;
                }
            }
            if let Some(g) = group {
                if proc.group.as_deref() != Some(g) {
                    continue;
                }
            }
            let matches = proc.search(pattern);
            if !matches.is_empty() {
                results.push((proc.name.clone(), matches));
            }
        }
        results
    }

    /// Stop a process by name.
    pub fn stop(&mut self, name: &str) -> Result<(), String> {
        self.processes
            .remove(name)
            .map(|_| ()) // Child is killed on drop
            .ok_or_else(|| format!("process '{name}' not found"))
    }

    /// List all running processes.
    #[allow(dead_code)] // TODO: expose via DSL
    pub fn list(&self) -> Vec<(&str, Option<&str>)> {
        self.processes
            .values()
            .map(|p| (p.name.as_str(), p.group.as_deref()))
            .collect()
    }
}
