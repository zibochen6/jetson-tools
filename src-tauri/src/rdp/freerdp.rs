use super::args;
use super::client::RdpClient;
use super::error::RdpError;
use super::process::{spawn_sidecar, RdpProcess};
use super::types::RdpConnectionConfig;

/// FreeRDP 3.x via the `sdl-freerdp` binary (independent native window).
/// Phase 4A uses the system-installed binary; Phase 4B will bundle it as a
/// Tauri sidecar (see DECISIONS ADR-029).
pub struct FreeRdpSidecarClient;

impl FreeRdpSidecarClient {
    const KNOWN_PATHS: &'static [&'static str] = &[
        "/opt/homebrew/bin/sdl-freerdp",
        "/usr/local/bin/sdl-freerdp",
        "/usr/bin/sdl-freerdp",
    ];

    /// Locate the binary: an explicit dev override first, then known
    /// Homebrew/system paths, then the shell `PATH`. Never scans the filesystem
    /// broadly, never installs anything (PRD §27).
    pub fn locate_binary(&self) -> Result<String, RdpError> {
        if let Ok(path) = std::env::var("RDP_BINARY_PATH") {
            if !path.is_empty() && std::path::Path::new(&path).is_file() {
                return Ok(path);
            }
        }
        for path in Self::KNOWN_PATHS {
            if std::path::Path::new(path).is_file() {
                return Ok(path.to_string());
            }
        }
        locate_in_path("sdl-freerdp").ok_or(RdpError::ClientNotFound)
    }

    /// Validate the binary reports a supported FreeRDP 3.x version (PRD §26:
    /// a present binary is not necessarily compatible).
    pub async fn preflight(&self, binary: &str) -> Result<(), RdpError> {
        let output = tokio::process::Command::new(binary)
            .arg("--version")
            .output()
            .await
            .map_err(RdpError::LaunchFailed)?;
        if !output.status.success() {
            return Err(RdpError::VersionUnsupported);
        }
        let text = String::from_utf8_lossy(&output.stdout);
        if !text.contains("3.") {
            return Err(RdpError::VersionUnsupported);
        }
        Ok(())
    }
}

impl RdpClient for FreeRdpSidecarClient {
    async fn launch(&self, config: &RdpConnectionConfig) -> Result<RdpProcess, RdpError> {
        let binary = self.locate_binary()?;
        self.preflight(&binary).await?;
        let title = format!("Jetson Remote — {}", config.host);
        let args = args::build(&binary, config, &title);
        spawn_sidecar(&args).await
    }
}

fn locate_in_path(name: &str) -> Option<String> {
    let path = std::env::var_os("PATH")?;
    for dir in std::env::split_paths(&path) {
        let candidate = dir.join(name);
        if candidate.is_file() {
            return Some(candidate.to_string_lossy().into_owned());
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn locate_binary_uses_env_override() {
        // Point at any existing file (e.g. the shell itself) to exercise the
        // override path without depending on FreeRDP being installed.
        let bin = "/bin/sh".to_string();
        // Unsafe to mutate env in parallel tests; scope-guard it.
        let old = std::env::var("RDP_BINARY_PATH").ok();
        std::env::set_var("RDP_BINARY_PATH", &bin);
        let found = FreeRdpSidecarClient.locate_binary().unwrap();
        std::env::remove_var("RDP_BINARY_PATH");
        if let Some(v) = old {
            std::env::set_var("RDP_BINARY_PATH", v);
        }
        assert_eq!(found, bin);
    }

    #[test]
    fn locate_in_path_finds_real_binary() {
        // `sh` is always on PATH on macOS/Linux.
        assert!(locate_in_path("sh").is_some());
    }
}
