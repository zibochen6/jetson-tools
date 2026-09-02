//! Remembered-device IPC surface (V0.3): read / write / forget the device
//! memory. Passwords cross this boundary only in `remember_device` — the
//! exact same trust as the connect flow — and are never returned by any
//! command, never logged, and never placed in argv.

use serde::Serialize;
use tauri::{AppHandle, State};

use super::connection::{ProbeError, ProbeErrorCode};
use crate::remember::{
    self, KeychainSecretStore, RememberedDevice, RememberedDeviceStore, SecretStore,
};

/// Status-shaped result of `get_remembered_device`. Has NO password field by
/// design — `has_password` is a Keychain probe the frontend uses to decide
/// between auto-reconnect and prefill-only.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RememberedDeviceStatus {
    pub host: String,
    pub username: String,
    pub has_password: bool,
}

fn device_store(app: &AppHandle) -> Result<RememberedDeviceStore, ProbeError> {
    RememberedDeviceStore::for_app(app)
        .map_err(|e| ProbeError::new(ProbeErrorCode::Unknown, format!("config dir: {e}")))
}

/// Last remembered device, or None on first launch. `has_password` queries the
/// OS keychain; a missing/unreadable secret degrades to `false` (prefill-only).
#[tauri::command]
pub fn get_remembered_device(
    app: AppHandle,
    secrets: State<'_, KeychainSecretStore>,
) -> Result<Option<RememberedDeviceStatus>, ProbeError> {
    let store = device_store(&app)?;
    let Some(device) = store.load() else {
        return Ok(None);
    };
    let has_password = secrets
        .get(remember::KEYCHAIN_SERVICE, &device.keychain_account())
        .is_some();
    Ok(Some(RememberedDeviceStatus {
        host: device.host,
        username: device.username,
        has_password,
    }))
}

/// Persist the device identity + password (called by the frontend right after
/// a successful device probe). Keychain write happens first: a crash between
/// the two writes leaves an orphan password entry rather than a device whose
/// "remembered" status silently lost its secret.
#[tauri::command]
pub fn remember_device(
    app: AppHandle,
    secrets: State<'_, KeychainSecretStore>,
    host: String,
    username: String,
    password: String,
) -> Result<(), ProbeError> {
    let store = device_store(&app)?;
    let device = RememberedDevice {
        host: host.clone(),
        username: username.clone(),
    };
    secrets
        .set(
            remember::KEYCHAIN_SERVICE,
            &device.keychain_account(),
            &password,
        )
        .map_err(ProbeError::from)?;
    store.save(&device).map_err(ProbeError::from)?;
    Ok(())
}

/// Delete the remembered device: Keychain entry (idempotent) + JSON record.
#[tauri::command]
pub fn forget_remembered_device(
    app: AppHandle,
    secrets: State<'_, KeychainSecretStore>,
) -> Result<(), ProbeError> {
    let store = device_store(&app)?;
    if let Some(device) = store.load() {
        secrets
            .delete(remember::KEYCHAIN_SERVICE, &device.keychain_account())
            .map_err(ProbeError::from)?;
    }
    store.clear().map_err(ProbeError::from)?;
    Ok(())
}