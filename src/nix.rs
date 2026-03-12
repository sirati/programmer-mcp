//! Nix environment detection and LSP fallback support.
//!
//! When a configured LSP command is not found in `PATH`, and nix with flakes
//! is available, we attempt to run the language server via
//! `nix run nixpkgs#{pkg} -- {args}`.

use tracing::{debug, info, warn};

// ── known LSP → nixpkgs attribute mappings ────────────────────────────────────

/// Returns the `nixpkgs` attribute path for a given LSP binary name.
/// Returns `None` if not in the known list — callers then fall back to the
/// command name itself.
fn nix_pkg_for_command(command: &str) -> Option<&'static str> {
    match command {
        "rust-analyzer" => Some("rust-analyzer"),
        "gopls" => Some("gopls"),
        "pyright" | "pyright-langserver" => Some("pyright"),
        "basedpyright" | "basedpyright-langserver" => Some("basedpyright"),
        "pylsp" | "pylsp-all" => Some("python3Packages.python-lsp-server"),
        "ruff-lsp" => Some("ruff-lsp"),
        "ruff" => Some("ruff"),
        "typescript-language-server" => Some("nodePackages.typescript-language-server"),
        "vscode-eslint-language-server"
        | "vscode-json-language-server"
        | "vscode-css-language-server"
        | "vscode-html-language-server" => Some("nodePackages.vscode-langservers-extracted"),
        "clangd" => Some("clang-tools"),
        "ccls" => Some("ccls"),
        "zls" => Some("zls"),
        "nil" => Some("nil"),
        "nixd" => Some("nixd"),
        "lua-language-server" => Some("lua-language-server"),
        "kotlin-language-server" => Some("kotlin-language-server"),
        "jdtls" | "jdt-language-server" => Some("jdt-language-server"),
        "ruby-lsp" => Some("rubyPackages.ruby-lsp"),
        "solargraph" => Some("rubyPackages.solargraph"),
        "metals" => Some("metals"),
        "haskell-language-server-wrapper" | "hls" => Some("haskell-language-server"),
        "elixir-ls" => Some("elixir-ls"),
        "erlang-ls" => Some("erlang-ls"),
        "omnisharp" => Some("omnisharp-roslyn"),
        "psalm" => Some("php83Packages.psalm"),
        "intelephense" => Some("nodePackages.intelephense"),
        "bash-language-server" => Some("nodePackages.bash-language-server"),
        "yaml-language-server" => Some("yaml-language-server"),
        "taplo" | "taplo-lsp" => Some("taplo"),
        "marksman" => Some("marksman"),
        "terraform-ls" => Some("terraform-ls"),
        "helm-ls" => Some("helm-ls"),
        "docker-langserver" => Some("nodePackages.dockerfile-language-server-nodejs"),
        "cmake-language-server" => Some("cmake-language-server"),
        "swift-frontend" => Some("swift"),
        _ => None,
    }
}

// ── nix environment state ──────────────────────────────────────────────────────

/// Cached result of nix detection.
#[derive(Debug, Clone)]
pub struct NixEnv {
    /// `nix` binary is present in `PATH`.
    pub available: bool,
    /// Nix flakes (`nix-command` + `flakes` experimental features) are enabled.
    pub flakes: bool,
}

impl NixEnv {
    /// Probe the environment once and return the result.
    pub async fn detect() -> Self {
        let available = is_nix_available().await;
        if !available {
            debug!("nix not found in PATH");
            return Self {
                available: false,
                flakes: false,
            };
        }
        let flakes = has_flakes().await;
        info!(flakes, "nix detected");
        Self { available, flakes }
    }

    /// Return `(command, args)` that runs `original_command original_args` via
    /// `nix run nixpkgs#{pkg} -- ...`, or `None` if nix+flakes are unavailable.
    pub fn fallback_command(
        &self,
        original_command: &str,
        original_args: &[String],
    ) -> Option<(String, Vec<String>)> {
        if !self.available || !self.flakes {
            return None;
        }

        // Use the known mapping, or fall back to the command name directly.
        let pkg = nix_pkg_for_command(original_command).unwrap_or(original_command);
        let mut args = vec![
            "run".to_string(),
            format!("nixpkgs#{pkg}"),
            "--".to_string(),
        ];
        args.extend(original_args.iter().cloned());

        info!(
            original_command,
            pkg, "using nix run fallback for missing LSP binary"
        );

        Some(("nix".to_string(), args))
    }
}

// ── detection helpers ──────────────────────────────────────────────────────────

/// Returns `true` if the `nix` binary can be found in `PATH`.
async fn is_nix_available() -> bool {
    match tokio::process::Command::new("nix")
        .arg("--version")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .await
    {
        Ok(s) => s.success(),
        Err(_) => false,
    }
}

/// Returns `true` if nix flakes (and `nix-command`) are enabled.
///
/// Detection strategy (in order):
/// 1. Check `~/.config/nix/nix.conf` and `/etc/nix/nix.conf` for the
///    `experimental-features` line.
/// 2. Try `nix flake --help` – if it exits 0 flakes are available.
async fn has_flakes() -> bool {
    // Fast path: check nix.conf files
    let conf_paths = nix_conf_paths();
    for path in &conf_paths {
        if let Ok(content) = tokio::fs::read_to_string(path).await {
            if conf_has_flakes(&content) {
                debug!(path = %path.display(), "flakes enabled via nix.conf");
                return true;
            }
        }
    }

    // Slow path: probe `nix flake --help`
    match tokio::process::Command::new("nix")
        .args(["flake", "--help"])
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .await
    {
        Ok(s) if s.success() => {
            debug!("flakes enabled (nix flake --help succeeded)");
            true
        }
        _ => {
            warn!("nix available but flakes appear disabled");
            false
        }
    }
}

/// Return the list of nix.conf paths to search.
fn nix_conf_paths() -> Vec<std::path::PathBuf> {
    let mut paths = vec![std::path::PathBuf::from("/etc/nix/nix.conf")];
    if let Ok(home) = std::env::var("HOME") {
        paths.push(std::path::PathBuf::from(home).join(".config/nix/nix.conf"));
    }
    if let Ok(xdg) = std::env::var("XDG_CONFIG_HOME") {
        paths.push(std::path::PathBuf::from(xdg).join("nix/nix.conf"));
    }
    paths
}

/// Parse a nix.conf content string and return `true` if both `nix-command` and
/// `flakes` appear in the `experimental-features` value.
fn conf_has_flakes(content: &str) -> bool {
    for line in content.lines() {
        let line = line.trim();
        if line.starts_with('#') {
            continue;
        }
        if let Some(rest) = line
            .strip_prefix("experimental-features")
            .map(|s| s.trim_start_matches([' ', '\t', '=']))
        {
            let has_nix_cmd = rest.split_whitespace().any(|f| f == "nix-command");
            let has_flakes = rest.split_whitespace().any(|f| f == "flakes");
            return has_nix_cmd && has_flakes;
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_conf_has_flakes() {
        let conf = "experimental-features = nix-command flakes\n";
        assert!(conf_has_flakes(conf));

        let conf = "experimental-features = nix-command\n";
        assert!(!conf_has_flakes(conf));

        let conf = "# experimental-features = nix-command flakes\n";
        assert!(!conf_has_flakes(conf));

        let conf = "experimental-features=nix-command flakes\n";
        assert!(conf_has_flakes(conf));
    }
}
