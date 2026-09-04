//! Remembered-device persistence (V0.3, multi-device in V0.4, identity-v3).
//!
//! Security split (PRD §29 / §67, ADR-006):
//! - Non-secret device identities live in `remembered.json` under the app
//!   config dir — a v3 list ordered most-recently-connected first.
//! - Each password lives ONLY in the OS secret store, keyed by service +
//!   account (`user@deviceId` in v3; legacy v2 used `user@host`). It never
//!   leaves this module as part of any serializable struct.
//!
//! Identity model (v3): one device = one `machine-id` + a required display
//! name + a mutable set of paths (LAN / Tailscale addresses). The address the
//! user types is only an entry point; after a successful probe the device's
//! current address list overwrites the stored paths.
//!
//! `SecretStore` is a trait so unit tests inject an in-memory fake; the
//! production implementation is a 0600 JSON file in the app config dir
//! (KI-020): the macOS Keychain was dropped because ad-hoc/unsigned builds
//! have no stable Keychain ACL identity, so macOS re-prompted for the login
//! password on every launch.
//!
//! File formats:
//! - v3: `{"version":3,"devices":[{"deviceId","username","displayName",
//!        "paths":[{"kind","address"}],"lastUsedPath","lastConnectedAt"}]}`
//! - v2 (legacy, read-only): `{"version":2,"devices":[{"host","username",
//!        "lastConnectedAt"}]}` — surfaced as deviceId=null,
//!   paths=[{kind:"lan",host}], displayName=null; merged into the matching
//!   machine-id entry on the next successful connect.
//! - v1 (legacy, read-only): a bare `{"host","username"}` object.

use std::fs;
use std::io::Write;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager};

use crate::device::types::DevicePath;

/// Service name scoping the remembered devices' passwords in the secret store.
pub const SECRET_SERVICE: &str = "com.jetsonremote.app.remembered-device";

/// App Support directory name; mirrors the bundle identifier in
/// `tauri.conf.json` (the store is constructed before an AppHandle exists).
const APP_SUPPORT_DIR: &str = "com.jetsonremote.app";

/// File name of the secret store inside the app config dir.
const SECRETS_FILE_NAME: &str = "secrets.json";

/// File name inside the app config dir holding the non-secret device list.
pub const FILE_NAME: &str = "remembered.json";

/// Current on-disk schema version.
const FORMAT_VERSION: u32 = 3;

/// Non-secret device identity. Deliberately has NO password field.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RememberedDevice {
    /// Stable identity (`/etc/machine-id`); `None` for legacy v2 entries.
    pub device_id: Option<String>,
    pub username: String,
    /// Required display name for v3 devices; `None` until the user names the
    /// device on its first connect (legacy v2 entries have none either).
    pub display_name: Option<String>,
    /// The device's current candidate addresses (LAN / Tailscale).
    #[serde(default)]
    pub paths: Vec<DevicePath>,
    /// The address the last successful connection used.
    #[serde(default)]
    pub last_used_path: Option<String>,
}

impl RememberedDevice {
    /// Secret-store account derived from device identity; unique per device
    /// so the list can hold many devices without collisions. v3 uses
    /// `user@deviceId`; legacy v2 entries keep `user@host` (their stored
    /// secret stays reachable until the merge migrates it).
    pub fn account(&self) -> String {
        match &self.device_id {
            Some(id) => format!("{}@{}", self.username, id),
            None => {
                let host = self
                    .paths
                    .first()
                    .map(|p| p.address.clone())
                    .unwrap_or_default();
                format!("{}@{}", self.username, host)
            }
        }
    }

    /// Every address this device may be reachable on.
    pub fn addresses(&self) -> Vec<&str> {
        self.paths.iter().map(|p| p.address.as_str()).collect()
    }

    /// True when this entry is a legacy v2 shape (no machine-id).
    pub fn is_legacy(&self) -> bool {
        self.device_id.is_none()
    }

    /// Same physical device? v3: same deviceId. Legacy: same username and a
    /// shared path address (or the exact host when the caller knows it).
    fn same_device(&self, device_id: Option<&str>, username: &str, hosts: &[&str]) -> bool {
        if self.username != username {
            return false;
        }
        match device_id {
            Some(id) => self.device_id.as_deref() == Some(id),
            None => self.device_id.is_none() && hosts.iter().any(|h| self.addresses().contains(h)),
        }
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

/* ---------------------------- on-disk shapes ---------------------------- */

/// One v3 list entry (camelCase on disk).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct DeviceEntry {
    device_id: Option<String>,
    username: String,
    display_name: Option<String>,
    #[serde(default)]
    paths: Vec<DevicePath>,
    #[serde(default)]
    last_used_path: Option<String>,
    /// Milliseconds since the Unix epoch; 0 = unknown (legacy migration).
    #[serde(default)]
    last_connected_at: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct DevicesFile {
    version: u32,
    devices: Vec<DeviceEntry>,
}

/// One v2 list entry (legacy, read-only).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct DeviceEntryV2 {
    host: String,
    username: String,
    #[serde(default)]
    last_connected_at: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct DevicesFileV2 {
    version: u32,
    devices: Vec<DeviceEntryV2>,
}

/// Legacy v1: a bare single-device object.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct DeviceEntryV1 {
    host: String,
    username: String,
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis().min(u64::MAX as u128) as u64)
        .unwrap_or(0)
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

    /// Every remembered device, most recently connected first. Missing or
    /// corrupt files yield an empty list; v2/v1 files are read transparently
    /// (deviceId=null until the device is re-connected and merged).
    pub fn load_all(&self) -> Vec<RememberedDevice> {
        self.entries_mru()
            .into_iter()
            .map(|e| RememberedDevice {
                device_id: e.device_id,
                username: e.username,
                display_name: e.display_name,
                paths: e.paths,
                last_used_path: e.last_used_path,
            })
            .collect()
    }

    /// Most recently connected device, if any.
    pub fn load(&self) -> Option<RememberedDevice> {
        self.load_all().into_iter().next()
    }

    /// Atomically persist (tmp + rename) so a crash can't leave a torn file.
    /// Always writes the v3 shape.
    fn write_entries(&self, devices: &[DeviceEntry]) -> Result<(), RememberError> {
        if let Some(dir) = self.file.parent() {
            fs::create_dir_all(dir).map_err(|e| RememberError::Io(e.to_string()))?;
        }
        let file = DevicesFile {
            version: FORMAT_VERSION,
            devices: devices.to_vec(),
        };
        let bytes =
            serde_json::to_vec(&file).map_err(|e| RememberError::Io(format!("serialize: {e}")))?;
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

    /// Raw entries (with timestamps) in MRU order — internal read path that
    /// understands v3, v2 and v1 shapes.
    fn entries_mru(&self) -> Vec<DeviceEntry> {
        let Ok(bytes) = fs::read(&self.file) else {
            return Vec::new();
        };
        let Ok(value) = serde_json::from_slice::<serde_json::Value>(&bytes) else {
            return Vec::new();
        };

        // v3 list format.
        if let Ok(file) = serde_json::from_value::<DevicesFile>(value.clone()) {
            if file.version >= 3 {
                let mut entries = file.devices;
                // Most recent first; stable sort keeps file order for equal stamps.
                entries.sort_by_key(|e| std::cmp::Reverse(e.last_connected_at));
                return entries;
            }
        }

        // v2 list format → v3 shape (deviceId null, single LAN path).
        if let Ok(file) = serde_json::from_value::<DevicesFileV2>(value.clone()) {
            let mut entries: Vec<DeviceEntry> = file
                .devices
                .into_iter()
                .map(|e| DeviceEntry {
                    device_id: None,
                    username: e.username,
                    display_name: None,
                    paths: vec![DevicePath::lan(e.host)],
                    last_used_path: None,
                    last_connected_at: e.last_connected_at,
                })
                .collect();
            entries.sort_by_key(|e| std::cmp::Reverse(e.last_connected_at));
            return entries;
        }

        // Legacy v1: a bare single-device object.
        match serde_json::from_value::<DeviceEntryV1>(value) {
            Ok(device) => vec![DeviceEntry {
                device_id: None,
                username: device.username,
                display_name: None,
                paths: vec![DevicePath::lan(device.host)],
                last_used_path: None,
                last_connected_at: 0,
            }],
            Err(_) => Vec::new(),
        }
    }

    /// Insert or refresh a device: it moves to the front with a fresh
    /// `lastConnectedAt`; every other entry is preserved untouched.
    /// Dedup: same deviceId (v3), or same username + shared address (legacy).
    pub fn upsert(&self, device: &RememberedDevice) -> Result<(), RememberError> {
        let hosts = device.addresses();
        let mut entries: Vec<DeviceEntry> = self
            .entries_mru()
            .into_iter()
            .filter(|e| {
                !e
                    .device_id
                    .as_deref()
                    .is_some_and(|id| device.device_id.as_deref() == Some(id))
                    && !(!e.device_id.is_some()
                        && device.device_id.is_none()
                        && e.same_device_legacy(&device.username, &hosts))
            })
            .collect();
        entries.insert(
            0,
            DeviceEntry {
                device_id: device.device_id.clone(),
                username: device.username.clone(),
                display_name: device.display_name.clone(),
                paths: device.paths.clone(),
                last_used_path: device.last_used_path.clone(),
                last_connected_at: now_ms(),
            },
        );
        self.write_entries(&entries)
    }

    /// Remove ONE device by identity; missing entries count as success.
    /// v3: by deviceId. Legacy: by username + host (any path match).
    pub fn remove(
        &self,
        device_id: Option<&str>,
        host: Option<&str>,
        username: &str,
    ) -> Result<(), RememberError> {
        let mut entries = self.entries_mru();
        let before = entries.len();
        entries.retain(|e| {
            let matched_v3 = device_id.is_some() && e.device_id.as_deref() == device_id;
            let matched_legacy = device_id.is_none()
                && e.device_id.is_none()
                && e.username == username
                && host.is_some_and(|h| e.paths.iter().any(|p| p.address == h));
            // A forget-by-deviceId also drops legacy entries of the same
            // username whose host matches (merged duplicates).
            let matched_merge = device_id.is_some()
                && e.device_id.is_none()
                && e.username == username
                && host.is_some_and(|h| e.paths.iter().any(|p| p.address == h));
            !(matched_v3 || matched_legacy || matched_merge)
        });
        if entries.len() == before {
            return Ok(()); // idempotent
        }
        if entries.is_empty() {
            // Nothing left: drop the file entirely (first-launch shape).
            return self.clear();
        }
        self.write_entries(&entries)
    }

    /// Legacy-v2 entries for the same username that share an address with
    /// `hosts` — the candidates whose secrets must be migrated into the v3
    /// `user@deviceId` account and then deleted (merge on first connect).
    pub fn legacy_merge_candidates(
        &self,
        username: &str,
        hosts: &[&str],
    ) -> Vec<RememberedDevice> {
        self.load_all()
            .into_iter()
            .filter(|d| d.is_legacy() && d.same_device(None, username, hosts))
            .collect()
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

impl DeviceEntry {
    /// Legacy-only same-device check (both sides have no deviceId).
    fn same_device_legacy(&self, username: &str, hosts: &[&str]) -> bool {
        self.device_id.is_none()
            && self.username == username
            && self.paths.iter().any(|p| hosts.contains(&p.address.as_str()))
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
/// platforms get the same implementation; the chmod is unix-specific.
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
/// always wins; an empty/missing one falls back to the OS secret store.
///
/// Identity resolution order:
/// 1. `device_id` present → the `user@deviceId` account, precise even with
///    several remembered devices. When that account has no secret yet (the
///    legacy merge is still in flight), falls back to the host lookup.
/// 2. Legacy v2 device (no deviceId) → match by username + host in paths.
///    Loopback (in-app tunnel, KI-021) is only attributable while exactly ONE
///    device is remembered; with several, the caller must type the password.
pub fn resolve_password(
    remembered: &RememberedDeviceStore,
    secrets: &dyn SecretStore,
    device_id: Option<&str>,
    host: &str,
    username: &str,
    provided: Option<&str>,
) -> Result<String, RememberError> {
    if let Some(p) = provided {
        if !p.is_empty() {
            return Ok(p.to_string());
        }
    }
    let devices = remembered.load_all();

    let device = if let Some(id) = device_id.filter(|i| !i.is_empty()) {
        // deviceId is precise; when its entry or secret is not there yet (a
        // legacy v2 device whose merge into `user@deviceId` is still in
        // flight), fall through to the host-based lookup below.
        match devices
            .iter()
            .find(|d| d.device_id.as_deref() == Some(id) && d.username == username)
            .filter(|d| secrets.get(SECRET_SERVICE, &d.account()).is_some())
        {
            Some(d) => d.clone(),
            None => match devices
                .iter()
                .find(|d| d.username == username && d.addresses().contains(&host))
            {
                Some(d) => d.clone(),
                None => return Err(RememberError::Missing),
            },
        }
    } else if host == "127.0.0.1" {
        // Tunnel mode: the wire host carries no identity of its own.
        match devices.as_slice() {
            [only] if only.username == username => only.clone(),
            _ => return Err(RememberError::Missing),
        }
    } else {
        match devices
            .iter()
            .find(|d| d.username == username && d.addresses().contains(&host))
        {
            Some(d) => d.clone(),
            None => return Err(RememberError::Missing),
        }
    };

    secrets
        .get(SECRET_SERVICE, &device.account())
        .ok_or(RememberError::Missing)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::device::types::DevicePath;
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
            self.0
                .lock()
                .unwrap()
                .insert((service.to_string(), account.to_string()), secret.to_string());
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

    fn paths(items: &[(&str, &str)]) -> Vec<DevicePath> {
        items
            .iter()
            .map(|(kind, address)| DevicePath {
                kind: kind.to_string(),
                address: address.to_string(),
            })
            .collect()
    }

    fn dev_v3(id: &str, username: &str, name: &str, addrs: &[&str]) -> RememberedDevice {
        RememberedDevice {
            device_id: Some(id.into()),
            username: username.into(),
            display_name: Some(name.into()),
            paths: addrs.iter().map(|a| DevicePath::lan(*a)).collect(),
            last_used_path: addrs.first().map(|a| a.to_string()),
        }
    }

    fn dev_legacy(host: &str, username: &str) -> RememberedDevice {
        RememberedDevice {
            device_id: None,
            username: username.into(),
            display_name: None,
            paths: vec![DevicePath::lan(host)],
            last_used_path: None,
        }
    }

    const DEV_HOST: &str = "192.168.100.164";
    const DEV_USER: &str = "seeed";
    const DEV_ID: &str = "5dbfb12400000000";

    fn temp_store() -> RememberedDeviceStore {
        let dir = temp_dir();
        let _ = fs::remove_dir_all(&dir);
        RememberedDeviceStore::new(dir)
    }

    #[test]
    fn upsert_roundtrip_and_clear() {
        let store = temp_store();
        let device = dev_v3(DEV_ID, DEV_USER, "mini", &[DEV_HOST]);

        assert_eq!(store.load_all(), Vec::<RememberedDevice>::new());
        store.upsert(&device).unwrap();
        assert_eq!(store.load_all(), vec![device.clone()]);
        assert_eq!(store.load(), Some(device.clone()));

        store.clear().unwrap();
        assert_eq!(store.load_all(), Vec::<RememberedDevice>::new());
        // idempotent
        store.clear().unwrap();
    }

    #[test]
    fn upsert_rewrites_paths_and_moves_to_front() {
        let store = temp_store();
        let a = dev_v3("id-a", "seeed", "A", &["10.0.0.1"]);
        let b = dev_v3("id-b", "seeed", "B", &["10.0.0.2"]);
        store.upsert(&a).unwrap();
        std::thread::sleep(std::time::Duration::from_millis(2));
        store.upsert(&b).unwrap();
        std::thread::sleep(std::time::Duration::from_millis(2));

        // Same device id, NEW address set: paths are overwritten (stale
        // addresses dropped), entry moves to the front, no duplicate.
        let a2 = dev_v3("id-a", "seeed", "A", &["10.0.0.9", "100.64.0.9"]);
        store.upsert(&a2).unwrap();
        let all = store.load_all();
        assert_eq!(all.len(), 2);
        assert_eq!(all[0], a2);
        assert_eq!(all[1], b);
    }

    #[test]
    fn v2_file_is_read_as_legacy_entries() {
        let dir = temp_dir();
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        fs::write(
            dir.join(FILE_NAME),
            br#"{"version":2,"devices":[{"host":"192.168.100.164","username":"seeed","lastConnectedAt":5},{"host":"10.0.0.7","username":"alice","lastConnectedAt":9}]}"#,
        )
        .unwrap();
        let store = RememberedDeviceStore::new(dir);
        assert_eq!(
            store.load_all(),
            vec![dev_legacy("10.0.0.7", "alice"), dev_legacy(DEV_HOST, DEV_USER)]
        );
        // Legacy account stays user@host so the old secret resolves.
        assert_eq!(
            store.load_all()[1].account(),
            "seeed@192.168.100.164"
        );
    }

    #[test]
    fn legacy_single_object_migrates_to_a_one_device_list() {
        let dir = temp_dir();
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        fs::write(
            dir.join(FILE_NAME),
            br#"{"host":"192.168.100.164","username":"seeed"}"#,
        )
        .unwrap();
        let store = RememberedDeviceStore::new(dir);
        assert_eq!(store.load_all(), vec![dev_legacy(DEV_HOST, DEV_USER)]);

        // The next write rewrites the file in v3 shape.
        store.upsert(&dev_v3("id-x", "alice", "X", &["10.0.0.5"])).unwrap();
        let raw = fs::read_to_string(store.file.clone()).unwrap();
        let value: serde_json::Value = serde_json::from_str(&raw).unwrap();
        assert_eq!(value["version"], 3);
        assert_eq!(value["devices"].as_array().unwrap().len(), 2);
        assert_eq!(store.load(), Some(dev_v3("id-x", "alice", "X", &["10.0.0.5"])));
    }

    #[test]
    fn corrupt_file_yields_an_empty_list() {
        let dir = temp_dir();
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join(FILE_NAME), b"{not json").unwrap();
        let store = RememberedDeviceStore::new(dir);
        assert_eq!(store.load_all(), Vec::<RememberedDevice>::new());
    }

    #[test]
    fn remove_deletes_only_the_named_device() {
        let store = temp_store();
        store.upsert(&dev_v3("id-a", "a", "A", &["10.0.0.1"])).unwrap();
        store.upsert(&dev_v3("id-b", "b", "B", &["10.0.0.2"])).unwrap();

        // v3 removal by deviceId.
        store.remove(Some("id-b"), None, "b").unwrap();
        assert_eq!(store.load_all(), vec![dev_v3("id-a", "a", "A", &["10.0.0.1"])]);

        // Unknown identity: idempotent success.
        store.remove(Some("id-z"), None, "nobody").unwrap();
        assert_eq!(store.load_all().len(), 1);

        // Removing the last device restores the first-launch shape.
        store.remove(Some("id-a"), None, "a").unwrap();
        assert_eq!(store.load_all(), Vec::<RememberedDevice>::new());
        assert!(!store.file.exists());
    }

    #[test]
    fn remove_also_drops_merged_legacy_duplicates() {
        let store = temp_store();
        store.upsert(&dev_legacy(DEV_HOST, DEV_USER)).unwrap();
        store.upsert(&dev_v3("id-x", "u", "X", &["10.0.0.5"])).unwrap();
        // Forget the v3 device while naming its old LAN host: the legacy
        // duplicate must go with it.
        store.remove(Some("id-x"), Some(DEV_HOST), "u").unwrap();
        assert_eq!(store.load_all(), vec![dev_legacy(DEV_HOST, DEV_USER)]);
    }

    #[test]
    fn legacy_merge_candidates_match_shared_addresses() {
        let store = temp_store();
        store.upsert(&dev_legacy("192.168.2.18", "seeed")).unwrap();
        store.upsert(&dev_legacy("10.0.0.7", "alice")).unwrap();
        store.upsert(&dev_v3(DEV_ID, "seeed", "robotics", &["192.168.2.19"])).unwrap();

        let candidates = store.legacy_merge_candidates("seeed", &["192.168.2.18", "192.168.2.19"]);
        assert_eq!(candidates, vec![dev_legacy("192.168.2.18", "seeed")]);
    }

    #[test]
    fn account_is_username_at_device_id() {
        let d = dev_v3("5dbfb124", "seeed", "mini", &["192.168.100.164"]);
        assert_eq!(d.account(), "seeed@5dbfb124");
        // Legacy keeps user@host.
        assert_eq!(dev_legacy("jetson.local", "seeed").account(), "seeed@jetson.local");
    }

    #[test]
    fn resolve_uses_typed_password_first() {
        let secrets = FakeSecretStore::default();
        let store = temp_store();
        store.upsert(&dev_v3(DEV_ID, DEV_USER, "mini", &[DEV_HOST])).unwrap();
        secrets
            .set(SECRET_SERVICE, &format!("{DEV_USER}@{DEV_ID}"), "stored")
            .unwrap();
        let pw = resolve_password(&store, &secrets, Some(DEV_ID), DEV_HOST, DEV_USER, Some("typed")).unwrap();
        assert_eq!(pw, "typed");
    }

    #[test]
    fn resolve_by_device_id_is_precise_with_several_devices() {
        let secrets = FakeSecretStore::default();
        let store = temp_store();
        let a = dev_v3("id-a", "seeed", "A", &["10.0.0.1"]);
        let b = dev_v3("id-b", "seeed", "B", &["10.0.0.2"]);
        store.upsert(&a).unwrap();
        store.upsert(&b).unwrap();
        secrets.set(SECRET_SERVICE, "seeed@id-a", "stored-a").unwrap();
        secrets.set(SECRET_SERVICE, "seeed@id-b", "stored-b").unwrap();

        let pw = resolve_password(&store, &secrets, Some("id-b"), "10.0.0.2", "seeed", None).unwrap();
        assert_eq!(pw, "stored-b");
        // Wire host may be ANY of the device's paths — the deviceId decides.
        let pw = resolve_password(&store, &secrets, Some("id-a"), "irrelevant-entry-host", "seeed", Some("")).unwrap();
        assert_eq!(pw, "stored-a");
        // Unknown deviceId + a host that matches NO remembered path → missing.
        assert!(matches!(
            resolve_password(&store, &secrets, Some("id-z"), "10.9.9.9", "seeed", None),
            Err(RememberError::Missing)
        ));
        // Unknown deviceId but the host matches a legacy/v3 path → the host
        // lookup still resolves (merge-in-flight race safety).
        let pw = resolve_password(&store, &secrets, Some("id-z"), "10.0.0.1", "seeed", None).unwrap();
        assert_eq!(pw, "stored-a");
    }

    #[test]
    fn resolve_device_id_falls_back_to_legacy_secret_before_merge() {
        // Race safety: the v3 entry exists (paths refreshed) but its secret
        // still lives under the legacy `user@host` account.
        let secrets = FakeSecretStore::default();
        let store = temp_store();
        store
            .upsert(&dev_v3(DEV_ID, DEV_USER, "mini", &[DEV_HOST]))
            .unwrap();
        secrets
            .set(SECRET_SERVICE, &format!("{DEV_USER}@{DEV_HOST}"), "legacy-secret")
            .unwrap();
        // A legacy entry for the same board makes the host lookup succeed.
        store.upsert(&dev_legacy(DEV_HOST, DEV_USER)).unwrap();
        let pw = resolve_password(&store, &secrets, Some(DEV_ID), DEV_HOST, DEV_USER, None).unwrap();
        assert_eq!(pw, "legacy-secret");
    }

    #[test]
    fn resolve_legacy_falls_back_by_path() {
        let secrets = FakeSecretStore::default();
        let store = temp_store();
        store.upsert(&dev_legacy(DEV_HOST, DEV_USER)).unwrap();
        store.upsert(&dev_legacy("10.0.0.7", "alice")).unwrap();
        secrets
            .set(SECRET_SERVICE, "seeed@192.168.100.164", "stored1")
            .unwrap();
        secrets
            .set(SECRET_SERVICE, "alice@10.0.0.7", "stored2")
            .unwrap();

        let pw = resolve_password(&store, &secrets, None, DEV_HOST, DEV_USER, None).unwrap();
        assert_eq!(pw, "stored1");
        let pw = resolve_password(&store, &secrets, None, "10.0.0.7", "alice", Some("")).unwrap();
        assert_eq!(pw, "stored2");
    }

    #[test]
    fn resolve_works_over_loopback_with_exactly_one_device() {
        // KI-021 tunnel: the wire host is 127.0.0.1 but the stored identity
        // is the LAN host — resolution must still find the stored secret.
        let secrets = FakeSecretStore::default();
        let store = temp_store();
        store.upsert(&dev_v3(DEV_ID, DEV_USER, "mini", &[DEV_HOST])).unwrap();
        secrets
            .set(SECRET_SERVICE, &format!("{DEV_USER}@{DEV_ID}"), "stored")
            .unwrap();
        let pw = resolve_password(&store, &secrets, None, "127.0.0.1", DEV_USER, None).unwrap();
        assert_eq!(pw, "stored");
    }

    #[test]
    fn resolve_refuses_loopback_when_several_devices_are_remembered() {
        // With more than one remembered device the loopback wire host is
        // ambiguous — the user must type the password.
        let secrets = FakeSecretStore::default();
        let store = temp_store();
        store.upsert(&dev_v3(DEV_ID, DEV_USER, "mini", &[DEV_HOST])).unwrap();
        store.upsert(&dev_v3("id-b", "alice", "B", &["10.0.0.7"])).unwrap();
        secrets
            .set(SECRET_SERVICE, &format!("{DEV_USER}@{DEV_ID}"), "stored")
            .unwrap();
        assert!(matches!(
            resolve_password(&store, &secrets, None, "127.0.0.1", DEV_USER, None),
            Err(RememberError::Missing)
        ));
    }

    #[test]
    fn resolve_rejects_an_unremembered_identity() {
        let secrets = FakeSecretStore::default();
        let store = temp_store();
        store.upsert(&dev_v3(DEV_ID, DEV_USER, "mini", &[DEV_HOST])).unwrap();
        secrets
            .set(SECRET_SERVICE, &format!("{DEV_USER}@{DEV_ID}"), "stored")
            .unwrap();
        // different host (non-loopback) must not consume any stored password
        assert!(matches!(
            resolve_password(&store, &secrets, None, "10.0.0.9", DEV_USER, None),
            Err(RememberError::Missing)
        ));
        // different username must not either
        assert!(matches!(
            resolve_password(&store, &secrets, Some(DEV_ID), DEV_HOST, "other", None),
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
        let store = temp_store();
        assert!(matches!(
            resolve_password(&store, &secrets, Some(DEV_ID), DEV_HOST, DEV_USER, None),
            Err(RememberError::Missing)
        ));
    }

    #[test]
    fn paths_helper_lists_addresses() {
        let d = RememberedDevice {
            device_id: Some("id".into()),
            username: "u".into(),
            display_name: Some("n".into()),
            paths: paths(&[("lan", "192.168.2.18"), ("tailscale", "100.114.170.49")]),
            last_used_path: None,
        };
        assert_eq!(d.addresses(), vec!["192.168.2.18", "100.114.170.49"]);
    }
}
