//! Trust data model (r2 §6.3/§6.13). Pure data + serde; no secrets.

use serde::{Deserialize, Serialize};

/// One SSH host key entry (algorithm + fingerprint). NEVER a private key.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HostKeyRecord {
    pub algorithm: String,
    pub fingerprint: String,
}

/// The Jetson product identity read AFTER a successful SSH login.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeviceIdentity {
    /// `/etc/machine-id` (may be missing or cloned across boards).
    pub machine_id: Option<String>,
    /// `/proc/device-tree/serial-number` — unique per module, survives reflash.
    pub serial: Option<String>,
}

/// Everything we remember about one logical device's trust state.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TrustRecord {
    /// Stable logical identity (serial preferred, machine-id fallback).
    pub logical_device_id: String,
    pub machine_id: Option<String>,
    pub serial: Option<String>,
    /// Keys this device has presented and we have accepted.
    pub trusted_host_keys: Vec<HostKeyRecord>,
    /// Host keys seen in the past (allowing "same IP, new key" detection).
    pub known_addresses: Vec<String>,
    pub first_seen_at: Option<u64>,
    pub last_seen_at: Option<u64>,
}

impl TrustRecord {
    pub fn matches_host_key(&self, fingerprint: &str) -> bool {
        self.trusted_host_keys
            .iter()
            .any(|k| k.fingerprint == fingerprint)
    }
}
