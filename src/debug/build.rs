use std::path::{Path, PathBuf};

use tokio::process::Command;

pub struct BuildOutcome {
    pub binary_path: Option<PathBuf>,
    pub errors: String,
}

impl BuildOutcome {
    pub fn success(&self) -> bool {
        self.binary_path.is_some()
    }
}

pub async fn run_cargo_build(project_root: &Path) -> BuildOutcome {
    let result = Command::new("cargo")
        .args(["build", "--message-format=short"])
        .current_dir(project_root)
        .output()
        .await;

    match result {
        Err(e) => BuildOutcome {
            binary_path: None,
            errors: format!("failed to spawn cargo: {e}"),
        },
        Ok(output) if !output.status.success() => {
            let stderr = String::from_utf8_lossy(&output.stderr);
            let stdout = String::from_utf8_lossy(&output.stdout);
            BuildOutcome {
                binary_path: None,
                errors: filter_errors(&stderr, &stdout),
            }
        }
        Ok(_) => BuildOutcome {
            binary_path: find_binary(project_root),
            errors: String::new(),
        },
    }
}

fn filter_errors(stderr: &str, stdout: &str) -> String {
    let combined = format!("{stderr}\n{stdout}");
    let errors: Vec<&str> = combined
        .lines()
        .filter(|l| l.contains("error[") || l.starts_with("error:") || l.contains("] error:"))
        .collect();

    if errors.is_empty() {
        combined.trim().to_string()
    } else {
        errors.join("\n")
    }
}

fn find_binary(project_root: &Path) -> Option<PathBuf> {
    let name = read_package_name(project_root)?;
    let binary = project_root.join("target").join("debug").join(&name);
    binary.exists().then_some(binary)
}

fn read_package_name(project_root: &Path) -> Option<String> {
    let content = std::fs::read_to_string(project_root.join("Cargo.toml")).ok()?;
    extract_package_name(&content)
}

fn extract_package_name(toml: &str) -> Option<String> {
    let mut in_package = false;
    for line in toml.lines() {
        let trimmed = line.trim();
        if trimmed == "[package]" {
            in_package = true;
            continue;
        }
        if in_package && trimmed.starts_with('[') {
            in_package = false;
        }
        if in_package && trimmed.starts_with("name") {
            if let Some(name) = trimmed.split('"').nth(1) {
                return Some(name.to_string());
            }
        }
    }
    None
}
