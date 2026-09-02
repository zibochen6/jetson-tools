use std::fmt;

use russh::keys::{HashAlg, PublicKeyOrCertificate};

pub const DEFAULT_PORT: u16 = 22;
pub const MAX_OUTPUT_BYTES: usize = 1 << 20; // 1 MiB

#[derive(Clone, serde::Deserialize)]
pub struct SshConnectionInput {
    pub host: String,
    #[serde(default = "default_port")]
    pub port: u16,
    pub username: String,
    /// `None` = use the remembered password from the OS secret store
    /// (resolved by `remember::resolve_password` at the command boundary).
    pub password: Option<String>,
}

pub fn default_port() -> u16 {
    DEFAULT_PORT
}

// Secret-safe Debug: never prints the password.
impl fmt::Debug for SshConnectionInput {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SshConnectionInput")
            .field("host", &self.host)
            .field("port", &self.port)
            .field("username", &self.username)
            .field("password", &"<redacted>")
            .finish()
    }
}

#[derive(Clone, Debug)]
pub struct SshConfig {
    pub connect_timeout: std::time::Duration,
    pub command_timeout: std::time::Duration,
    /// Long timeout for multi-minute commands (e.g. apt provisioning).
    pub provision_timeout: std::time::Duration,
    pub max_output_bytes: usize,
}

impl Default for SshConfig {
    fn default() -> Self {
        Self {
            connect_timeout: std::time::Duration::from_secs(8),
            command_timeout: std::time::Duration::from_secs(15),
            provision_timeout: std::time::Duration::from_secs(15 * 60),
            max_output_bytes: MAX_OUTPUT_BYTES,
        }
    }
}

/// Non-secret host-key metadata surfaced to the user for TOFU trust decisions.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HostKeyInfo {
    pub host: String,
    pub port: u16,
    pub algorithm: String,
    pub fingerprint: String, // OpenSSH-style "SHA256:<base64>"
}

impl HostKeyInfo {
    pub fn from_key(host: &str, port: u16, key: &PublicKeyOrCertificate) -> Self {
        let (algorithm, fingerprint) = match key {
            PublicKeyOrCertificate::PublicKey { key, .. } => (
                key.algorithm().to_string(),
                key.fingerprint(HashAlg::Sha256).to_string(),
            ),
            // Host certificates are effectively never presented by sshd as the
            // client-facing key; marker is deliberately non-trusting (see DECISIONS).
            PublicKeyOrCertificate::Certificate(cert) => {
                (cert.algorithm().to_string(), "SHA256:unknown".to_string())
            }
        };
        Self {
            host: host.to_string(),
            port,
            algorithm,
            fingerprint,
        }
    }

    pub fn key_id(&self) -> String {
        format!("{}:{}", self.host, self.port)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn input_debug_redacts_password() {
        let input = SshConnectionInput {
            host: "192.168.100.164".into(),
            port: 22,
            username: "seeed".into(),
            password: Some("s3cret".into()),
        };
        let debug = format!("{input:?}");
        assert!(!debug.contains("s3cret"));
        assert!(debug.contains("<redacted>"));

        let none_input = SshConnectionInput {
            host: "h".into(),
            port: 22,
            username: "u".into(),
            password: None,
        };
        // Redacted even when absent — the debug shape stays constant.
        assert!(format!("{none_input:?}").contains("<redacted>"));
    }
}
