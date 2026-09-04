//! Remembered-device IPC surface (identity-v3): read / write / forget the
//! device memory. Passwords cross this boundary only in `remember_device` —
//! the exact same trust as the connect flow — and are never returned by any
//! command, never logged, and never placed in argv.
//!
//! v3 identity: one device = one machine-id + a required display name + a
//! mutable path list. `remember_device` also merges legacy v2 `user@host`
//! entries into the machine-id identity on the first successful connect, so
//! the "one IP = one device" duplicates disappear.

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, State};

use super::connection::{ProbeError, ProbeErrorCode};
use crate::device::types::DevicePath;
use crate::remember::{
    self, FileSecretStore, RememberedDevice, RememberedDeviceStore, SecretStore,
};

/// Status-shaped row of `get_remembered_devices`. Has NO password field by
/// design — `has_password` is a secret-store probe the frontend uses to decide
/// between auto-reconnect and prefill-only.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RememberedDeviceStatus {
    /// Stable identity; `null` for legacy v2 entries (not yet re-connected).
    pub device_id: Option<String>,
    pub username: String,
    /// The user-chosen display name; `null` until the device is named.
    pub display_name: Option<String>,
    /// Current known candidate addresses (LAN / Tailscale).
    pub paths: Vec<DevicePath>,
    /// The address the last successful connection used.
    pub last_used_path: Option<String>,
    pub has_password: bool,
}

fn device_store(app: &AppHandle) -> Result<RememberedDeviceStore, ProbeError> {
    RememberedDeviceStore::for_app(app)
        .map_err(|e| ProbeError::new(ProbeErrorCode::Unknown, format!("config dir: {e}")))
}

/// Every remembered device, most recently connected first. `has_password`
/// queries the OS secret store per device; a missing/unreadable secret
/// degrades to `false` (prefill-only).
#[tauri::command]
pub fn get_remembered_devices(
    app: AppHandle,
    secrets: State<'_, FileSecretStore>,
) -> Result<Vec<RememberedDeviceStatus>, ProbeError> {
    let store = device_store(&app)?;
    Ok(store
        .load_all()
        .into_iter()
        .map(|device| {
            let has_password = secrets
                .get(remember::SECRET_SERVICE, &device.account())
                .is_some();
            RememberedDeviceStatus {
                device_id: device.device_id,
                username: device.username,
                display_name: device.display_name,
                paths: device.paths,
                last_used_path: device.last_used_path,
                has_password,
            }
        })
        .collect())
}

/// IPC input of `remember_device`.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RememberDeviceInput {
    pub device_id: Option<String>,
    pub username: String,
    /// Required for v3 devices; the naming screen collects it before this is
    /// called. `None` keeps the previous name (blank-password reconnect).
    pub display_name: Option<String>,
    /// The device's current addresses as reported by detect.sh. Replaces the
    /// stored list (stale addresses are dropped).
    #[serde(default)]
    pub paths: Vec<DevicePath>,
    /// The address the user typed this round (kept as `lastUsedPath` and used
    /// to match legacy v2 entries for the merge).
    #[serde(default)]
    pub entry_host: Option<String>,
    /// The typed password. Empty/None = keep the stored secret (the backend
    /// already authenticated with it).
    pub password: Option<String>,
}

/// Persist one device's identity + password (called by the frontend right
/// after a successful device probe / naming). Upsert semantics: the device
/// moves to the front of the remembered list; every other remembered device is
/// preserved.
///
/// Merge: legacy v2 entries of the same user whose host is one of this
/// device's addresses (or the typed entry host) are folded in — their secret
/// is copied to `user@deviceId` when the v3 account has none, then the legacy
/// entry + secret are deleted.
#[tauri::command]
pub fn remember_device(
    app: AppHandle,
    secrets: State<'_, FileSecretStore>,
    input: RememberDeviceInput,
) -> Result<(), ProbeError> {
    let store = device_store(&app)?;
    let device_id = input
        .device_id
        .as_deref()
        .map(str::trim)
        .filter(|id| !id.is_empty())
        .map(str::to_string);
    let display_name = input
        .display_name
        .as_deref()
        .map(str::trim)
        .filter(|n| !n.is_empty())
        .map(str::to_string);

    // The entry host is always a candidate path (this round's input).
    let mut paths = input.paths;
    if let Some(entry) = input.entry_host.as_deref().filter(|h| !h.is_empty()) {
        if !paths.iter().any(|p| p.address == entry) {
            paths.push(DevicePath::lan(entry));
        }
    }

    let account = match &device_id {
        Some(id) => format!("{}@{}", input.username, id),
        None => {
            let host = paths
                .first()
                .map(|p| p.address.clone())
                .ok_or_else(|| {
                    ProbeError::new(
                        ProbeErrorCode::Unknown,
                        "remember_device needs a device id or at least one address",
                    )
                })?;
            format!("{}@{}", input.username, host)
        }
    };

    // 1. Merge legacy v2 duplicates: copy their secret into the v3 account
    //    (when the v3 account has none), then drop entry + secret.
    let host_refs: Vec<&str> = paths.iter().map(|p| p.address.as_str()).collect();
    for legacy in store.legacy_merge_candidates(&input.username, &host_refs) {
        if let Some(old_secret) = secrets.get(remember::SECRET_SERVICE, &legacy.account()) {
            if secrets.get(remember::SECRET_SERVICE, &account).is_none() {
                secrets
                    .set(remember::SECRET_SERVICE, &account, &old_secret)
                    .map_err(ProbeError::from)?;
                eprintln!(
                    "[jr-flow] merged legacy secret {} -> {}",
                    legacy.account(),
                    account
                );
            }
        }
        secrets
            .delete(remember::SECRET_SERVICE, &legacy.account())
            .map_err(ProbeError::from)?;
        store
            .remove(None, legacy.paths.first().map(|p| p.address.as_str()), &input.username)
            .map_err(ProbeError::from)?;
    }

    // 2. Write the secret first (a crash between the two writes leaves an
    //    orphan password rather than a device whose "remembered" status
    //    silently lost its secret). Empty password = keep the stored one.
    if let Some(pw) = input.password.as_deref().filter(|p| !p.is_empty()) {
        secrets
            .set(remember::SECRET_SERVICE, &account, pw)
            .map_err(ProbeError::from)?;
    }

    // 3. Upsert the identity (paths overwrite; MRU front).
    let device = RememberedDevice {
        device_id,
        username: input.username.clone(),
        display_name,
        paths,
        last_used_path: input.entry_host,
    };
    store.upsert(&device).map_err(ProbeError::from)?;
    Ok(())
}

/// Delete ONE remembered device by identity: secret-store entry (idempotent)
/// + its list record. Other remembered devices are untouched.
///
/// Identity: deviceId when present (also drops merged legacy duplicates of
/// the same user); otherwise the legacy host+username pair.
#[tauri::command]
pub fn forget_remembered_device(
    app: AppHandle,
    secrets: State<'_, FileSecretStore>,
    device_id: Option<String>,
    host: Option<String>,
    username: String,
) -> Result<(), ProbeError> {
    let store = device_store(&app)?;

    // Collect every secret account this forget must remove: the v3 account
    // plus legacy duplicates sharing the named host.
    let device_id = device_id.as_deref().map(str::trim).filter(|id| !id.is_empty());
    let host = host.as_deref().map(str::trim).filter(|h| !h.is_empty());

    let mut accounts: Vec<String> = Vec::new();
    for d in store.load_all() {
        let same_username = d.username == username;
        let matches_v3 = device_id.is_some() && d.device_id.as_deref() == device_id;
        let matches_legacy = device_id.is_none()
            && d.is_legacy()
            && host.is_some_and(|h| d.addresses().contains(&h));
        let matches_merged = device_id.is_some()
            && d.is_legacy()
            && host.is_some_and(|h| d.addresses().contains(&h));
        if same_username && (matches_v3 || matches_legacy || matches_merged) {
            accounts.push(d.account());
        }
    }
    if accounts.is_empty() {
        // Unknown identity: derive the v3 account so a stored-but-unlisted
        // secret still gets cleaned up (idempotent).
        if let Some(id) = device_id {
            accounts.push(format!("{username}@{id}"));
        } else if let Some(h) = host {
            accounts.push(format!("{username}@{h}"));
        }
    }

    for account in &accounts {
        secrets
            .delete(remember::SECRET_SERVICE, account)
            .map_err(ProbeError::from)?;
    }
    store
        .remove(device_id, host, &username)
        .map_err(ProbeError::from)?;
    Ok(())
}
