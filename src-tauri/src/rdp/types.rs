use std::fmt;

use serde::{Deserialize, Serialize};

/// IPC shape from the frontend. Deliberately minimal — the frontend supplies
/// only the connection target; the safe FreeRDP invocation (cert policy,
/// clipboard, dynamic resolution) is built entirely in Rust (PRD §33).
#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RdpConnectionRequest {
    pub host: String,
    #[serde(default = "default_rdp_port")]
    pub port: u16,
    pub username: String,
    /// `None` = use the remembered password from the OS secret store
    /// (resolved by `remember::resolve_password` at the command boundary).
    pub password: Option<String>,
}

pub fn default_rdp_port() -> u16 {
    3389
}

// Secret-safe Debug: never prints the password.
impl fmt::Debug for RdpConnectionRequest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RdpConnectionRequest")
            .field("host", &self.host)
            .field("port", &self.port)
            .field("username", &self.username)
            .field("password", &"<redacted>")
            .finish()
    }
}

/// Internal connection configuration. `port` is NOT hardcoded to 3389 inside
/// the client — Phase 5 will point these at a local SSH-forwarded port.
#[derive(Clone)]
pub struct RdpConnectionConfig {
    pub host: String,
    pub port: u16,
    pub username: String,
    pub password: String,
    pub dynamic_resolution: bool,
    pub clipboard: bool,
}

impl From<RdpConnectionRequest> for RdpConnectionConfig {
    /// The caller (command boundary) resolves `password` from the OS secret
    /// store first; a None surviving to this point degrades to empty rather
    /// than leaking or crashing.
    fn from(r: RdpConnectionRequest) -> Self {
        Self {
            host: r.host,
            port: r.port,
            username: r.username,
            password: r.password.unwrap_or_default(),
            dynamic_resolution: true,
            clipboard: true,
        }
    }
}

impl fmt::Debug for RdpConnectionConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RdpConnectionConfig")
            .field("host", &self.host)
            .field("port", &self.port)
            .field("username", &self.username)
            .field("password", &"<redacted>")
            .field("dynamic_resolution", &self.dynamic_resolution)
            .field("clipboard", &self.clipboard)
            .finish()
    }
}

/// Observably-honest process status (PRD §18): a spawned process is "running",
/// not "connected" — we cannot confirm RDP authentication without deeper
/// integration. `Exited` carries an OS exit code when available (crash / close).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum RdpStatus {
    NotRunning,
    Running,
    Exited {
        exit_code: Option<i32>,
        error: Option<String>,
    },
}

/// Outcome of a launch request.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum RdpLaunchResult {
    Opened,
    AlreadyRunning,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn request_debug_redacts_password() {
        let req = RdpConnectionRequest {
            host: "192.168.100.164".into(),
            port: 3389,
            username: "seeed".into(),
            password: Some("s3cret".into()),
        };
        let dbg = format!("{req:?}");
        assert!(!dbg.contains("s3cret"));
        assert!(dbg.contains("<redacted>"));
    }

    #[test]
    fn config_debug_redacts_password() {
        let config = RdpConnectionConfig {
            host: "h".into(),
            port: 3389,
            username: "u".into(),
            password: "s3cret".into(),
            dynamic_resolution: true,
            clipboard: true,
        };
        assert!(!format!("{config:?}").contains("s3cret"));
    }

    #[test]
    fn request_port_defaults_to_3389() {
        let req: RdpConnectionRequest =
            serde_json::from_str(r#"{"host":"h","username":"u","password":"p"}"#).unwrap();
        assert_eq!(req.port, 3389);
    }

    #[test]
    fn request_password_defaults_to_missing() {
        let req: RdpConnectionRequest =
            serde_json::from_str(r#"{"host":"h","username":"u"}"#).unwrap();
        assert_eq!(req.password, None);
    }
}
