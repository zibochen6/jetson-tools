use std::sync::{Arc, Mutex};

use russh::client::Handler;
use russh::keys::PublicKeyOrCertificate;

use super::types::HostKeyInfo;

/// Pure TOFU decision: returns `(accept, changed)`.
/// - `expected = Some(e)` and `e == seen` → `(true, false)`  (trusted, matches)
/// - `expected = Some(e)` and `e != seen` → `(false, true)`  (changed key)
/// - `expected = None`                     → `(false, false)` (unknown key)
pub fn tofu_decision(expected: Option<&str>, seen: &str) -> (bool, bool) {
    match expected {
        Some(e) if e == seen => (true, false),
        Some(_) => (false, true),
        None => (false, false),
    }
}

/// Shared state between the handshake and the caller for host-key TOFU.
#[derive(Default)]
pub struct TrustState {
    pub expected_fingerprint: Option<String>,
    pub captured: Option<HostKeyInfo>,
}

pub struct ClientHandler {
    pub host: String,
    pub port: u16,
    pub trust: Arc<Mutex<TrustState>>,
}

impl Handler for ClientHandler {
    type Error = russh::Error;

    async fn check_server_key(
        &mut self,
        server_public_key: &PublicKeyOrCertificate,
    ) -> Result<bool, Self::Error> {
        let info = HostKeyInfo::from_key(&self.host, self.port, server_public_key);
        let (accept, changed) = tofu_decision(
            self.trust.lock().unwrap().expected_fingerprint.as_deref(),
            &info.fingerprint,
        );
        if !accept {
            self.trust.lock().unwrap().captured = Some(info);
            // Distinguishing unknown vs changed is the caller's job (it knows
            // the expected value); here we just reject the handshake.
            let _ = changed;
        }
        Ok(accept)
    }
}

#[cfg(test)]
mod tests {
    use super::tofu_decision;

    #[test]
    fn known_key_matches() {
        assert_eq!(
            tofu_decision(Some("SHA256:aaa"), "SHA256:aaa"),
            (true, false)
        );
    }

    #[test]
    fn unknown_key() {
        assert_eq!(tofu_decision(None, "SHA256:aaa"), (false, false));
    }

    #[test]
    fn changed_key() {
        assert_eq!(
            tofu_decision(Some("SHA256:aaa"), "SHA256:bbb"),
            (false, true)
        );
    }
}
