//! Remembered-device persistence (V0.3).
//!
//! Security split (PRD §29 / §67, ADR-006):
//! - Non-secret device identity (host + username) lives in `remembered.json`
//!   under the app config dir.
//! - The password lives ONLY in the OS secret store (macOS Keychain), keyed by
//!   service + account. It is never written to a plain file and never leaves
//!   this module as part of any serializable struct.
//!
//! `SecretStore` is a trait so unit tests inject an in-memory fake; the
//! production implementation wraps `keyring` (Apple-native Keychain).

use std::fs;
use std::io::Write;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager};

/// Keychain service name for the remembered device's password.
pub const KEYCHAIN_SERVICE: &str = "com.jetsonremote.app.remembered-device";

/// File name inside the app config dir holding the non-secret device identity.
pub const FILE_NAME: &str = "remembered.json";

/// Non-secret device identity. Deliberately has NO password field.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RememberedDevice {
    pub host: String,
    pub username: String,
}

impl RememberedDevice {
    /// Keychain account derived from device identity; unique per device so
    /// a device list could be supported later without a schema change.
    pub fn keychain_account(&self) -> String {
        format!("{}@{}", self.username, self.host)
    }
}

/// Errors from this module. `Missing` is the only one the frontend gets a
/// dedicated product error for ("stored password isn't available").
#[derive(Debug, thiserror::Error)]
pub enum RememberError {
    #[error("no stored password is available for this device")]
    Missing,
    #[error("secure storage error: {0}")]
    Secret(String),
    #[error("device memory error: {0}")]
    Io(String),
}

/// JSON file persistence for the non-secret part of the memory.
#[derive(Debug, Clone)]
pub struct RememberedDeviceStore {
    file: PathBuf,
}

impl RememberedDeviceStore {
    pub fn new(config_dir: PathBuf) -> Self {
        Self {
            file: config_dir.join(FILE_NAME),
        }
    }

    /// Resolve the store for the running app instance.
    pub fn for_app(app: &AppHandle) -> Result<Self, String> {
        let dir = app.path().app_config_dir().map_err(|e| e.to_string())?;
        Ok(Self::new(dir))
    }

    /// Last remembered device, or None when absent / corrupt / first launch.
    pub fn load(&self) -> Option<RememberedDevice> {
        let bytes = fs::read(&self.file).ok()?;
        serde_json::from_slice(&bytes).ok()
    }

    /// Atomically persist (tmp + rename) so a crash can't leave a torn file.
    pub fn save(&self, device: &RememberedDevice) -> Result<(), RememberError> {
        if let Some(dir) = self.file.parent() {
            fs::create_dir_all(dir).map_err(|e| RememberError::Io(e.to_string()))?;
        }
        let bytes = serde_json::to_vec(device)
            .map_err(|e| RememberError::Io(format!("serialize: {e}")))?;
        let tmp = self.file.with_extension("json.tmp");
        {
            let mut f = fs::File::create(&tmp)
                .map_err(|e| RememberError::Io(format!("create tmp: {e}")))?;
            f.write_all(&bytes)
                .map_err(|e| RememberError::Io(format!("write tmp: {e}")))?;
            f.sync_all()
                .map_err(|e| RememberError::Io(format!("sync tmp: {e}")))?;
        }
        fs::rename(&tmp, &self.file)
            .map_err(|e| RememberError::Io(format!("rename: {e}")))?;
        Ok(())
    }

    /// Remove the remembered record; a missing file counts as success.
    pub fn clear(&self) -> Result<(), RememberError> {
        match fs::remove_file(&self.file) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(RememberError::Io(e.to_string())),
        }
    }
}

/// Secret-storage abstraction: production = OS keychain, tests = in-memory.
pub trait SecretStore: Send + Sync {
    /// Stored secret, or None when absent (or unreadable).
    fn get(&self, service: &str, account: &str) -> Option<String>;
    fn set(&self, service: &str, account: &str, secret: &str) -> Result<(), RememberError>;
    fn delete(&self, service: &str, account: &str) -> Result<(), RememberError>;
}

#[cfg(target_os = "macos")]
fn secret_err(e: keyring::Error) -> RememberError {
    RememberError::Secret(e.to_string())
}

/// Production macOS backend via the `keyring` crate (Apple Keychain).
/// Non-macOS builds get the no-op fallback below: auto-reconnect degrades
/// gracefully to "type the password" instead of breaking.
#[cfg(target_os = "macos")]
#[derive(Debug, Default)]
pub struct KeychainSecretStore;

#[cfg(target_os = "macos")]
impl SecretStore for KeychainSecretStore {
    fn get(&self, service: &str, account: &str) -> Option<String> {
        let entry = keyring::Entry::new(service, account).ok()?;
        match entry.get_password() {
            Ok(pw) => Some(pw),
            Err(keyring::Error::NoEntry) => None,
            // Any other failure degrades to "no stored password": the user can
            // still type the password, so surfacing an error adds no value.
            Err(_) => None,
        }
    }

    fn set(&self, service: &str, account: &str, secret: &str) -> Result<(), RememberError> {
        let entry = keyring::Entry::new(service, account).map_err(secret_err)?;
        entry.set_password(secret).map_err(secret_err)
    }

    fn delete(&self, service: &str, account: &str) -> Result<(), RememberError> {
        let entry = keyring::Entry::new(service, account).map_err(secret_err)?;
        match entry.delete_credential() {
            Ok(()) => Ok(()),
            // Idempotent by design — forgetting twice is not an error.
            Err(keyring::Error::NoEntry) => Ok(()),
            Err(e) => Err(secret_err(e)),
        }
    }
}

#[cfg(not(target_os = "macos"))]
#[derive(Debug, Default)]
pub struct KeychainSecretStore;

#[cfg(not(target_os = "macos"))]
impl SecretStore for KeychainSecretStore {
    fn get(&self, _service: &str, _account: &str) -> Option<String> {
        None
    }

    fn set(&self, _service: &str, _account: &str, _secret: &str) -> Result<(), RememberError> {
        Ok(())
    }

    fn delete(&self, _service: &str, _account: &str) -> Result<(), RememberError> {
        Ok(())
    }
}

/// Resolve the password to use for a connection. A typed (non-empty) password
/// always wins; an empty/missing one falls back to the OS secret store for the
/// remembered device.
///
/// The Keychain account comes from `remembered.json`, NOT from the wire host:
/// in dev tunnel mode the wire host is loopback while the stored identity is
/// the LAN host the user typed. The single remembered device is the source of
/// truth (KI-004 tunnel workaround, V0.3).
pub fn resolve_password(
    remembered: &RememberedDeviceStore,
    secrets: &dyn SecretStore,
    host: &str,
    username: &str,
    provided: Option<&str>,
) -> Result<String, RememberError> {
    if let Some(p) = provided {
        if !p.is_empty() {
            return Ok(p.to_string());
        }
    }
    let device = remembered.load().ok_or(RememberError::Missing)?;
    // Guard: never reuse the stored password for a different identity. The
    // loopback exception admits dev tunnel mode, where the wire host is
    // 127.0.0.1 regardless of the LAN host typed by the user.
    if device.username != username || (device.host != host && host != "127.0.0.1") {
        return Err(RememberError::Missing);
    }
    secrets
        .get(KEYCHAIN_SERVICE, &device.keychain_account())
        .ok_or(RememberError::Missing)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use std::sync::Mutex;

    /// In-memory SecretStore for tests.
    #[derive(Default)]
    struct FakeSecretStore(Mutex<HashMap<(String, String), String>>);

    /// Every call gets its own directory — remember tests run on parallel
    /// threads and must not share filesystem state.
    fn temp_dir() -> PathBuf {
        static COUNTER: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let n = COUNTER.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        std::env::temp_dir().join(format!(
            "jr-remember-test-{}-{}",
            std::process::id(),
            n
        ))
    }

    impl SecretStore for FakeSecretStore {
        fn get(&self, service: &str, account: &str) -> Option<String> {
            self.0
                .lock()
                .unwrap()
                .get(&(service.to_string(), account.to_string()))
                .cloned()
        }

        fn set(&self, service: &str, account: &str, secret: &str) -> Result<(), RememberError> {
            self.0.lock().unwrap().insert(
                (service.to_string(), account.to_string()),
                secret.to_string(),
            );
            Ok(())
        }

        fn delete(&self, service: &str, account: &str) -> Result<(), RememberError> {
            self.0
                .lock()
                .unwrap()
                .remove(&(service.to_string(), account.to_string()));
            Ok(())
        }
    }

    fn store_with_device(dir: PathBuf, device: Option<&RememberedDevice>) -> RememberedDeviceStore {
        let store = RememberedDeviceStore::new(dir);
        let _ = store.clear();
        if let Some(d) = device {
            store.save(d).unwrap();
        }
        store
    }

    const DEV: &str = "192.168.100.164";

    fn dev() -> RememberedDevice {
        RememberedDevice {
            host: DEV.into(),
            username: "seeed".into(),
        }
    }

    fn temp_store() -> RememberedDeviceStore {
        store_with_device(temp_dir(), Some(&dev()))
    }

    #[test]
    fn store_roundtrip_and_clear() {
        let dir = temp_dir();
        let _ = fs::remove_dir_all(&dir);
        let store = RememberedDeviceStore::new(dir.clone());
        let device = RememberedDevice {
            host: "192.168.100.164".into(),
            username: "seeed".into(),
        };

        assert_eq!(store.load(), None);
        store.save(&device).unwrap();
        assert_eq!(store.load(), Some(device.clone()));

        store.clear().unwrap();
        assert_eq!(store.load(), None);
        // idempotent
        store.clear().unwrap();
    }

    #[test]
    fn store_returns_none_on_corrupt_file() {
        let dir = temp_dir();
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join(FILE_NAME), b"{not json").unwrap();
        let store = RememberedDeviceStore::new(dir);
        assert_eq!(store.load(), None);
    }

    #[test]
    fn keychain_account_is_username_at_host() {
        let d = RememberedDevice {
            host: "jetson.local".into(),
            username: "seeed".into(),
        };
        assert_eq!(d.keychain_account(), "seeed@jetson.local");
    }

    #[test]
    fn resolve_uses_typed_password_first() {
        let secrets = FakeSecretStore::default();
        let store = temp_store();
        secrets
            .set(KEYCHAIN_SERVICE, "seeed@192.168.100.164", "stored")
            .unwrap();
        let pw = resolve_password(&store, &secrets, DEV, "seeed", Some("typed")).unwrap();
        assert_eq!(pw, "typed");
    }

    #[test]
    fn resolve_falls_back_to_stored() {
        let secrets = FakeSecretStore::default();
        let store = temp_store();
        secrets
            .set(KEYCHAIN_SERVICE, "seeed@192.168.100.164", "stored")
            .unwrap();
        let pw = resolve_password(&store, &secrets, DEV, "seeed", None).unwrap();
        assert_eq!(pw, "stored");
        // empty string also falls back
        let pw = resolve_password(&store, &secrets, DEV, "seeed", Some("")).unwrap();
        assert_eq!(pw, "stored");
    }

    #[test]
    fn resolve_works_over_loopback_in_dev_tunnel_mode() {
        // KI-004 tunnel: the wire host is 127.0.0.1 but the stored identity
        // is the LAN host — resolution must still find the Keychain entry.
        let secrets = FakeSecretStore::default();
        let store = temp_store();
        secrets
            .set(KEYCHAIN_SERVICE, "seeed@192.168.100.164", "stored")
            .unwrap();
        let pw = resolve_password(&store, &secrets, "127.0.0.1", "seeed", None).unwrap();
        assert_eq!(pw, "stored");
    }

    #[test]
    fn resolve_rejects_a_different_device_identity() {
        let secrets = FakeSecretStore::default();
        let store = temp_store();
        secrets
            .set(KEYCHAIN_SERVICE, "seeed@192.168.100.164", "stored")
            .unwrap();
        // different host (non-loopback) must not consume the stored password
        assert!(matches!(
            resolve_password(&store, &secrets, "10.0.0.9", "seeed", None),
            Err(RememberError::Missing)
        ));
        // different username must not either
        assert!(matches!(
            resolve_password(&store, &secrets, DEV, "other", None),
            Err(RememberError::Missing)
        ));
    }

    #[test]
    fn resolve_missing_is_an_error() {
        let secrets = FakeSecretStore::default();
        let store = store_with_device(temp_dir(), None);
        assert!(matches!(
            resolve_password(&store, &secrets, DEV, "seeed", None),
            Err(RememberError::Missing)
        ));
    }
}