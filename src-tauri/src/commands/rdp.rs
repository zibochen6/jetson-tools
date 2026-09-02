use std::ffi::c_void;

use serde::Serialize;
use tauri::{AppHandle, Manager, State};

use crate::rdp::client::RdpClient;
use crate::rdp::error::{map_rdp_error, RdpError, RdpIpcError};
use crate::rdp::freerdp::FreeRdpSidecarClient;
use crate::rdp::manager::RdpProcessManager;
use crate::rdp::session::RdpSessionManager;
use crate::rdp::types::{RdpConnectionConfig, RdpConnectionRequest, RdpLaunchResult, RdpStatus};
use crate::remember::{self, FileSecretStore, RememberedDeviceStore};
use crate::tunnel::{TunnelError, TunnelManager};

/// DEV-only backend selector: `RDP_ENGINE=sidecar` forces the Phase 4A sidecar;
/// anything else (the default) uses the embedded libfreerdp native surface.
enum Engine {
    Embedded,
    Sidecar,
}

fn engine() -> Engine {
    match std::env::var("RDP_ENGINE").as_deref() {
        Ok("sidecar") => Engine::Sidecar,
        _ => Engine::Embedded,
    }
}

fn window_ns_window(app: &AppHandle) -> Result<*mut c_void, RdpError> {
    #[cfg(target_os = "macos")]
    {
        let win = app.get_webview_window("main").ok_or(RdpError::Unknown)?;
        win.ns_window()
            .map(|w| w.cast())
            .map_err(|_| RdpError::Unknown)
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = app;
        Err(RdpError::Unknown)
    }
}

/// Launch the desktop for a prepared Jetson. Returns `AlreadyRunning` rather
/// than opening a second surface (PRD §16). The frontend sends only the typed
/// connection request; Rust builds the safe invocation. A missing password is
/// resolved from the OS secret store (V0.3 auto-reconnect) — it never crosses
/// back into the frontend. The RDP plane rides the in-app loopback tunnel
/// (KI-021).
#[tauri::command]
pub async fn launch_remote_desktop(
    app: AppHandle,
    secrets: State<'_, FileSecretStore>,
    tunnels: State<'_, TunnelManager>,
    sidecar: State<'_, RdpProcessManager>,
    embedded: State<'_, RdpSessionManager>,
    mut request: RdpConnectionRequest,
) -> Result<RdpLaunchResult, RdpIpcError> {
    if sidecar.is_running() || embedded.is_running() {
        return Ok(RdpLaunchResult::AlreadyRunning);
    }

    // A typed password wins; None (or empty) falls back to the remembered
    // one. Missing → typed error; the frontend asks the user for a password.
    // The fallback resolves the secret-store account from remembered.json so
    // tunnel mode (wire host rewritten to loopback, KI-021) works too.
    let remembered_store =
        RememberedDeviceStore::for_app(&app).map_err(|_| map_rdp_error(RdpError::Unknown))?;
    let password = remember::resolve_password(
        &remembered_store,
        &*secrets,
        &request.host,
        &request.username,
        request.password.as_deref(),
    )
    .map_err(|_| map_rdp_error(RdpError::PasswordMissing))?;
    request.password = Some(password.clone());

    // Route the RDP plane through the in-app loopback tunnel (system ssh).
    let manager = tunnels.inner().clone();
    let app2 = app.clone();
    let host = request.host.clone();
    let username = request.username.clone();
    let endpoints = tokio::task::spawn_blocking(move || {
        manager.ensure(
            &app2,
            &host,
            crate::ssh::types::DEFAULT_PORT,
            &username,
            &password,
        )
    })
    .await
    .map_err(|_| map_rdp_error(RdpError::Unknown))?
    .map_err(map_tunnel_rdp_error)?;
    request.host = endpoints.host;
    request.port = endpoints.rdp_port;

    let config = RdpConnectionConfig::from(request);
    eprintln!(
        "[jr-flow] rdp launch start host={} port={}",
        config.host, config.port
    );

    match engine() {
        Engine::Embedded => {
            let ns_window = window_ns_window(&app).map_err(map_rdp_error)?;
            embedded.launch(&config, ns_window).map_err(map_rdp_error)?;
        }
        Engine::Sidecar => {
            let process = FreeRdpSidecarClient
                .launch(&config)
                .await
                .map_err(map_rdp_error)?;
            sidecar.set(process);
        }
    }
    Ok(RdpLaunchResult::Opened)
}

/// Gracefully terminate the active desktop. Idempotent; the remote Xorg/XFCE
/// session is left alive (PRD §24).
#[tauri::command]
pub async fn close_remote_desktop(
    sidecar: State<'_, RdpProcessManager>,
    embedded: State<'_, RdpSessionManager>,
) -> Result<(), RdpIpcError> {
    match engine() {
        Engine::Embedded => embedded.close().await.map_err(map_rdp_error),
        Engine::Sidecar => sidecar.close().await.map_err(map_rdp_error),
    }
}

/// Current process status — the frontend polls this to detect close/exit.
#[tauri::command]
pub fn rdp_status(
    sidecar: State<'_, RdpProcessManager>,
    embedded: State<'_, RdpSessionManager>,
) -> RdpStatus {
    match engine() {
        Engine::Embedded => embedded.status(),
        Engine::Sidecar => sidecar.status(),
    }
}

/* ------------------------------------------------------------------ */
/* Multi-device sessions (V0.4)                                        */
/* ------------------------------------------------------------------ */

/// Points reserved at the top of the window for the webview tab bar; focused
/// sessions mount their native view below this strip. MUST match `TAB_BAR_H`
/// in the frontend (`SessionTabBar`).
pub const SESSION_TAB_BAR_INSET: f64 = 44.0;

/// Password resolution + tunnel routing shared by the legacy launch and the
/// keyed session launch (identical behavior; extracted to avoid drift).
async fn prepare_request(
    app: &AppHandle,
    secrets: &FileSecretStore,
    tunnels: &TunnelManager,
    request: &mut RdpConnectionRequest,
) -> Result<(), RdpIpcError> {
    // A typed password wins; None (or empty) falls back to the remembered
    // one. Missing → typed error; the frontend asks the user for a password.
    // The fallback resolves the secret-store account from remembered.json so
    // tunnel mode (wire host rewritten to loopback, KI-021) works too.
    let remembered_store =
        RememberedDeviceStore::for_app(app).map_err(|_| map_rdp_error(RdpError::Unknown))?;
    let password = remember::resolve_password(
        &remembered_store,
        secrets,
        &request.host,
        &request.username,
        request.password.as_deref(),
    )
    .map_err(|_| map_rdp_error(RdpError::PasswordMissing))?;
    request.password = Some(password.clone());

    // Route the RDP plane through the in-app loopback tunnel (system ssh).
    let manager = tunnels.clone();
    let app2 = app.clone();
    let host = request.host.clone();
    let username = request.username.clone();
    let endpoints = tokio::task::spawn_blocking(move || {
        manager.ensure(
            &app2,
            &host,
            crate::ssh::types::DEFAULT_PORT,
            &username,
            &password,
        )
    })
    .await
    .map_err(|_| map_rdp_error(RdpError::Unknown))?
    .map_err(map_tunnel_rdp_error)?;
    request.host = endpoints.host;
    request.port = endpoints.rdp_port;
    Ok(())
}

/// Launch (or re-focus) ONE device's desktop session inside the multi-session
/// manager (V0.4). The session stays alive in the background when another tab
/// is focused, so switching back never reconnects. Same password/tunnel
/// security envelope as `launch_remote_desktop`.
#[tauri::command]
pub async fn launch_session(
    app: AppHandle,
    secrets: State<'_, FileSecretStore>,
    tunnels: State<'_, TunnelManager>,
    embedded: State<'_, RdpSessionManager>,
    session_id: String,
    mut request: RdpConnectionRequest,
) -> Result<RdpLaunchResult, RdpIpcError> {
    if matches!(engine(), Engine::Sidecar) {
        // Dev-only sidecar engine predates multi-session; keep it single.
        return Err(map_rdp_error(RdpError::Unknown));
    }
    if session_id.is_empty() {
        return Err(map_rdp_error(RdpError::Unknown));
    }
    if embedded.is_running_keyed(&session_id) {
        // Already connected: just bring it to the screen.
        let ns_window = window_ns_window(&app).map_err(map_rdp_error)?;
        embedded
            .focus(&session_id, ns_window, SESSION_TAB_BAR_INSET)
            .map_err(map_rdp_error)?;
        return Ok(RdpLaunchResult::AlreadyRunning);
    }

    prepare_request(&app, &secrets, &tunnels, &mut request).await?;

    let config = RdpConnectionConfig::from(request);
    eprintln!(
        "[jr-flow] rdp session launch id={} host={} port={}",
        session_id, config.host, config.port
    );
    let ns_window = window_ns_window(&app).map_err(map_rdp_error)?;
    embedded
        .launch_keyed(&session_id, &config, ns_window, SESSION_TAB_BAR_INSET)
        .map_err(map_rdp_error)?;
    Ok(RdpLaunchResult::Opened)
}

/// Quick-switch the on-screen desktop without reconnecting (V0.4 tab bar).
/// `sessionId: null` hides every session so the webview home is visible;
/// the RDP connections keep running in the background.
#[tauri::command]
pub async fn focus_session(
    app: AppHandle,
    embedded: State<'_, RdpSessionManager>,
    session_id: Option<String>,
) -> Result<(), RdpIpcError> {
    if matches!(engine(), Engine::Sidecar) {
        return Err(map_rdp_error(RdpError::Unknown));
    }
    match session_id {
        None => {
            embedded.hide_all();
            Ok(())
        }
        Some(id) => {
            let ns_window = window_ns_window(&app).map_err(map_rdp_error)?;
            embedded
                .focus(&id, ns_window, SESSION_TAB_BAR_INSET)
                .map_err(map_rdp_error)
        }
    }
}

/// Close ONE device's desktop session (tab "×"). Other sessions are
/// untouched. Idempotent; the remote Xorg/XFCE session is left alive.
#[tauri::command]
pub async fn close_session(
    embedded: State<'_, RdpSessionManager>,
    session_id: String,
) -> Result<(), RdpIpcError> {
    match engine() {
        Engine::Embedded => embedded
            .close_keyed(&session_id)
            .await
            .map_err(map_rdp_error),
        Engine::Sidecar => Err(map_rdp_error(RdpError::Unknown)),
    }
}

/// One session's status (frontend polls per active tab).
#[tauri::command]
pub fn session_status(embedded: State<'_, RdpSessionManager>, session_id: String) -> RdpStatus {
    embedded.status_keyed(&session_id)
}

/// IPC row for `all_session_statuses`.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionStatusEntry {
    pub session_id: String,
    pub status: RdpStatus,
}

/// Every session's status in one round-trip (tab-bar status dots + exit
/// detection for all connected devices).
#[tauri::command]
pub fn all_session_statuses(embedded: State<'_, RdpSessionManager>) -> Vec<SessionStatusEntry> {
    embedded
        .all_statuses()
        .into_iter()
        .map(|(session_id, status)| SessionStatusEntry { session_id, status })
        .collect()
}

fn map_tunnel_rdp_error(e: TunnelError) -> RdpIpcError {
    use crate::rdp::error::RdpErrorCode;
    let (code, message) = match e {
        TunnelError::AuthFailed => (
            RdpErrorCode::RdpAuthenticationFailed,
            "Authentication failed".to_string(),
        ),
        TunnelError::Unreachable => (
            RdpErrorCode::RdpConnectionFailed,
            "Could not reach the device".to_string(),
        ),
        TunnelError::Setup(m) => (RdpErrorCode::RdpUnknown, format!("Secure tunnel: {m}")),
    };
    RdpIpcError { code, message }
}
