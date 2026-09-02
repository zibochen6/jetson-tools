//! Remembered-device persistence (V0.3).
//!
//! Security split (PRD §29 / §67, ADR-006):
//! - Non-secret device identity (host + username) lives in `remembered.json`
//!   under the app config dir.
//! - The password lives ONLY in the OS secret store, keyed by service +
//!   account. It never leaves this module as part of any serializable struct.
//!
//! `SecretStore` is a trait so unit tests inject an in-memory fake; the
//! production implementation is a 0600 JSON file in the app config dir
//! (KI-020): the macOS Keychain was dropped because ad-hoc/unsigned builds
//! have no stable Keychain ACL identity, so macOS re-prompted for the login
//! password on every launch.

use std::fs;
use std::io::Write;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager};

/// Service name scoping the remembered device's password in the secret store.
pub const SECRET_SERVICE: &str = "com.jetsonremote.app.remembered-device";

/// App Support directory name; mirrors the bundle identifier in
/// `tauri.conf.json` (the store is constructed before an AppHandle exists).
const APP_SUPPORT_DIR: &str = "com.jetsonremote.app";

/// File name of the secret store inside the app config dir.
const SECRETS_FILE_NAME: &str = "secrets.json";

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
    /// Secret-store account derived from device identity; unique per device
    /// so a device list could be supported later without a schema change.
    pub fn account(&self) -> String {
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
        let bytes =
            serde_json::to_vec(device).map_err(|e| RememberError::Io(format!("serialize: {e}")))?;
        let tmp = self.file.with_extension("json.tmp");
        {
            let mut f = fs::File::create(&tmp)
                .map_err(|e| RememberError::Io(format!("create tmp: {e}")))?;
            f.write_all(&bytes)
                .map_err(|e| RememberError::Io(format!("write tmp: {e}")))?;
            f.sync_all()
                .map_err(|e| RememberError::Io(format!("sync tmp: {e}")))?;
        }
        fs::rename(&tmp, &self.file).map_err(|e| RememberError::Io(format!("rename: {e}")))?;
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

/// Secret-storage abstraction: production = 0600 file (KI-020), tests = in-memory.
pub trait SecretStore: Send + Sync {
    /// Stored secret, or None when absent (or unreadable).
    fn get(&self, service: &str, account: &str) -> Option<String>;
    fn set(&self, service: &str, account: &str, secret: &str) -> Result<(), RememberError>;
    fn delete(&self, service: &str, account: &str) -> Result<(), RememberError>;
}

/// Production backend: a 0600 JSON file in the app config dir. Never prompts
/// (KI-020) and behaves identically for signed and ad-hoc builds. Non-unix
/// platforms get the same implementation; only the chmod is unix-specific.
#[derive(Debug, Default)]
pub struct FileSecretStore;

impl FileSecretStore {
    /// `~/Library/Application Support/<bundle id>/secrets.json` on macOS.
    /// `JR_SECRETS_FILE` overrides the location (tests).
    fn file_path() -> Result<PathBuf, RememberError> {
        if let Ok(p) = std::env::var("JR_SECRETS_FILE") {
            if !p.is_empty() {
                return Ok(PathBuf::from(p));
            }
        }
        let home = std::env::var_os("HOME")
            .filter(|h| !h.is_empty())
            .ok_or_else(|| RememberError::Secret("HOME is not set".into()))?;
        Ok(PathBuf::from(home)
            .join("Library/Application Support")
            .join(APP_SUPPORT_DIR)
            .join(SECRETS_FILE_NAME))
    }

    fn load_map(
        path: &PathBuf,
    ) -> std::collections::BTreeMap<String, std::collections::BTreeMap<String, String>> {
        let Ok(bytes) = fs::read(path) else {
            return Default::default();
        };
        serde_json::from_slice(&bytes).unwrap_or_default()
    }

    fn save_map(
        path: &PathBuf,
        map: &std::collections::BTreeMap<String, std::collections::BTreeMap<String, String>>,
    ) -> Result<(), RememberError> {
        if let Some(dir) = path.parent() {
            fs::create_dir_all(dir)
                .map_err(|e| RememberError::Secret(format!("create dir: {e}")))?;
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let _ = fs::set_permissions(dir, fs::Permissions::from_mode(0o700));
            }
        }
        let bytes = serde_json::to_vec(map)
            .map_err(|e| RememberError::Secret(format!("serialize: {e}")))?;
        let tmp = path.with_extension("json.tmp");
        {
            let f = fs::File::create(&tmp)
                .map_err(|e| RememberError::Secret(format!("create: {e}")))?;
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let _ = f.set_permissions(fs::Permissions::from_mode(0o600));
            }
            let mut f = f;
            f.write_all(&bytes)
                .map_err(|e| RememberError::Secret(format!("write: {e}")))?;
            f.sync_all()
                .map_err(|e| RememberError::Secret(format!("sync: {e}")))?;
        }
        fs::rename(&tmp, path).map_err(|e| RememberError::Secret(format!("rename: {e}")))?;
        Ok(())
    }
}

impl SecretStore for FileSecretStore {
    fn get(&self, service: &str, account: &str) -> Option<String> {
        let path = Self::file_path().ok()?;
        Self::load_map(&path)
            .get(service)
            .and_then(|m| m.get(account))
            .cloned()
    }

    fn set(&self, service: &str, account: &str, secret: &str) -> Result<(), RememberError> {
        let path = Self::file_path()?;
        let mut map = Self::load_map(&path);
        map.entry(service.to_string())
            .or_default()
            .insert(account.to_string(), secret.to_string());
        Self::save_map(&path, &map)
    }

    fn delete(&self, service: &str, account: &str) -> Result<(), RememberError> {
        let path = Self::file_path()?;
        let mut map = Self::load_map(&path);
        if let Some(m) = map.get_mut(service) {
            m.remove(account);
        }
        // Idempotent by design — forgetting twice is not an error.
        Self::save_map(&path, &map)
    }
}

/// Resolve the password to use for a connection. A typed (non-empty) password
/// always wins; an empty/missing one falls back to the OS secret store for the
/// remembered device.
///
/// The secret-store account comes from `remembered.json`, NOT from the wire
/// host: the wire host may be loopback (in-app tunnel, KI-021) while the
/// stored identity is the LAN host the user typed. The single remembered
/// device is the source of truth.
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
        .get(SECRET_SERVICE, &device.account())
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
        std::env::temp_dir().join(format!("jr-remember-test-{}-{}", std::process::id(), n))
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
    fn account_is_username_at_host() {
        let d = RememberedDevice {
            host: "jetson.local".into(),
            username: "seeed".into(),
        };
        assert_eq!(d.account(), "seeed@jetson.local");
    }

    #[test]
    fn resolve_uses_typed_password_first() {
        let secrets = FakeSecretStore::default();
        let store = temp_store();
        secrets
            .set(SECRET_SERVICE, "seeed@192.168.100.164", "stored")
            .unwrap();
        let pw = resolve_password(&store, &secrets, DEV, "seeed", Some("typed")).unwrap();
        assert_eq!(pw, "typed");
    }

    #[test]
    fn resolve_falls_back_to_stored() {
        let secrets = FakeSecretStore::default();
        let store = temp_store();
        secrets
            .set(SECRET_SERVICE, "seeed@192.168.100.164", "stored")
            .unwrap();
        let pw = resolve_password(&store, &secrets, DEV, "seeed", None).unwrap();
        assert_eq!(pw, "stored");
        // empty string also falls back
        let pw = resolve_password(&store, &secrets, DEV, "seeed", Some("")).unwrap();
        assert_eq!(pw, "stored");
    }

    #[test]
    fn resolve_works_over_loopback_in_tunnel_mode() {
        // KI-021 tunnel: the wire host is 127.0.0.1 but the stored identity
        // is the LAN host — resolution must still find the stored secret.
        let secrets = FakeSecretStore::default();
        let store = temp_store();
        secrets
            .set(SECRET_SERVICE, "seeed@192.168.100.164", "stored")
            .unwrap();
        let pw = resolve_password(&store, &secrets, "127.0.0.1", "seeed", None).unwrap();
        assert_eq!(pw, "stored");
    }

    #[test]
    fn resolve_rejects_a_different_device_identity() {
        let secrets = FakeSecretStore::default();
        let store = temp_store();
        secrets
            .set(SECRET_SERVICE, "seeed@192.168.100.164", "stored")
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
    fn file_secret_store_roundtrip_and_permissions() {
        // Isolated location via the test-only env override.
        let dir = temp_dir();
        fs::create_dir_all(&dir).unwrap();
        let file = dir.join("secrets.json");
        std::env::set_var("JR_SECRETS_FILE", &file);
        let store = FileSecretStore;
        assert_eq!(store.get(SECRET_SERVICE, "seeed@h"), None);
        store.set(SECRET_SERVICE, "seeed@h", "s3cret").unwrap();
        assert_eq!(
            store.get(SECRET_SERVICE, "seeed@h").as_deref(),
            Some("s3cret")
        );
        // 0600 on the secrets file
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = fs::metadata(&file).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o600);
        }
        store.delete(SECRET_SERVICE, "seeed@h").unwrap();
        assert_eq!(store.get(SECRET_SERVICE, "seeed@h"), None);
        // idempotent
        store.delete(SECRET_SERVICE, "seeed@h").unwrap();
        std::env::remove_var("JR_SECRETS_FILE");
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
