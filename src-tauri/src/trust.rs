use std::collections::HashMap;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::ssh::types::HostKeyInfo;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StoredHost {
    pub algorithm: String,
    pub fingerprint: String,
}

#[derive(Default, Serialize, Deserialize)]
struct TrustStore {
    hosts: HashMap<String, StoredHost>,
}

/// Persistent TOFU trust store (`hosts.json` in the app config dir).
/// This is non-secret security metadata only — never contains credentials.
pub struct TrustStoreFile {
    path: PathBuf,
    store: TrustStore,
}

impl TrustStoreFile {
    pub fn load(dir: PathBuf) -> Result<Self, String> {
        let path = dir.join("hosts.json");
        let store = match std::fs::read_to_string(&path) {
            Ok(text) => serde_json::from_str(&text).unwrap_or_default(),
            Err(_) => TrustStore::default(),
        };
        Ok(Self { path, store })
    }

    pub fn get_fingerprint(&self, host: &str, port: u16) -> Option<String> {
        self.get(host, port).map(|h| h.fingerprint)
    }

    pub fn get(&self, host: &str, port: u16) -> Option<StoredHost> {
        self.store.hosts.get(&key_id(host, port)).cloned()
    }

    /// Insert (or overwrite — used by the explicit "Replace" flow) a host key.
    pub fn save(&mut self, key: &HostKeyInfo) -> Result<(), String> {
        self.store.hosts.insert(
            key.key_id(),
            StoredHost {
                algorithm: key.algorithm.clone(),
                fingerprint: key.fingerprint.clone(),
            },
        );
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        let text = serde_json::to_string_pretty(&self.store).map_err(|e| e.to_string())?;
        std::fs::write(&self.path, text).map_err(|e| e.to_string())
    }
}

fn key_id(host: &str, port: u16) -> String {
    format!("{host}:{port}")
}
