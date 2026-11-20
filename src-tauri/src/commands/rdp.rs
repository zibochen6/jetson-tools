use std::ffi::c_void;
use std::time::Duration;

use serde::Serialize;
use tauri::{AppHandle, Manager, State};

use crate::rdp::client::RdpClient;
use crate::rdp::error::{map_rdp_error, RdpError, RdpIpcError};
use crate::rdp::freerdp::FreeRdpSidecarClient;
use crate::rdp::manager::RdpProcessManager;
use crate::rdp::session::{RdpSessionManager, SessionLaunchMode};
use crate::rdp::types::{RdpConnectionConfig, RdpConnectionRequest, RdpLaunchResult, RdpStatus};
use crate::remember::{self, FileSecretStore, RememberedDeviceStore};
use crate::ssh::client as ssh;
use crate::ssh::executor::RemoteExecutor;
use crate::ssh::types::{SshConfig, SshConnectionInput};
use crate::trust::TrustStoreFile;
use crate::tunnel::{TunnelEndpoints, TunnelError, TunnelManager};

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

/// Stable per-device key for the tunnel: `username@deviceId` when known,
/// `username@host` otherwise (legacy).
fn device_key(host: &str, device_id: Option<&str>, username: &str) -> String {
    match device_id.filter(|i| !i.is_empty()) {
        Some(id) => format!("{username}@{id}"),
        None => format!("{username}@{host}"),
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
    #[cfg(target_os = "linux")]
    {
        // The opaque handle is the GtkOverlay installed by `.setup()` (see
        // `rdp::linux_view`). Missing means the reparent bootstrap failed.
        let _ = app;
        crate::rdp::linux_view::surface_handle().ok_or(RdpError::Unknown)
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
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
        request.device_id.as_deref(),
        &request.host,
        &request.username,
        request.password.as_deref(),
    )
    .map_err(|_| map_rdp_error(RdpError::PasswordMissing))?;
    request.password = Some(password.clone());

    // Route the RDP plane through the in-app loopback tunnel (system ssh),
    // keyed by the stable device identity.
    let certificate_name = request.host.clone();
    let manager = tunnels.inner().clone();
    let app2 = app.clone();
    let host = request.host.clone();
    let username = request.username.clone();
    let device_key = device_key(
        &request.host,
        request.device_id.as_deref(),
        &request.username,
    );
    let endpoints = tokio::task::spawn_blocking(move || {
        manager.ensure(
            &app2,
            &host,
            crate::ssh::types::DEFAULT_PORT,
            &device_key,
            &username,
            &password,
        )
    })
    .await
    .map_err(|_| map_rdp_error(RdpError::Unknown))?
    .map_err(map_tunnel_rdp_error)?;
    request.host = endpoints.host;
    request.port = endpoints.rdp_port;

    let mut config = RdpConnectionConfig::from(request);
    config.certificate_name = certificate_name;
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
///
/// `(async)` here is load-bearing, not cosmetic. A plain `#[tauri::command]` on
/// a non-async fn runs its body inline on the IPC (main) thread, and this
/// command takes the session-manager lock. A manager method that holds that same
/// lock across a `dispatch_sync(main)` (focus/refocus → attach_input) would then
/// wait for the main thread while the main thread waits for the lock — a
/// permanent, unrecoverable freeze (P1-11). `(async)` moves the body to a tokio
/// thread, keeping the main thread out of that cycle. Same reasoning applies to
/// `session_status` and `all_session_statuses`, which the frontend polls at 1 Hz.
#[tauri::command(async)]
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
) -> Result<TunnelEndpoints, RdpIpcError> {
    // A typed password wins; None (or empty) falls back to the remembered
    // one. Missing → typed error; the frontend asks the user for a password.
    // The fallback resolves the secret-store account from remembered.json so
    // tunnel mode (wire host rewritten to loopback, KI-021) works too.
    let remembered_store =
        RememberedDeviceStore::for_app(app).map_err(|_| map_rdp_error(RdpError::Unknown))?;
    let password = remember::resolve_password(
        &remembered_store,
        secrets,
        request.device_id.as_deref(),
        &request.host,
        &request.username,
        request.password.as_deref(),
    )
    .map_err(|_| map_rdp_error(RdpError::PasswordMissing))?;
    request.password = Some(password.clone());

    // Route the RDP plane through the in-app loopback tunnel (system ssh),
    // keyed by the stable device identity so both addresses of one Jetson
    // share a tunnel while two Jetsons never do.
    let certificate_name = request.host.clone();
    let manager = tunnels.clone();
    let app2 = app.clone();
    let host = request.host.clone();
    let username = request.username.clone();
    let device_key = device_key(
        &request.host,
        request.device_id.as_deref(),
        &request.username,
    );
    let endpoints = tokio::task::spawn_blocking(move || {
        manager.ensure(
            &app2,
            &host,
            crate::ssh::types::DEFAULT_PORT,
            &device_key,
            &username,
            &password,
        )
    })
    .await
    .map_err(|_| map_rdp_error(RdpError::Unknown))?
    .map_err(map_tunnel_rdp_error)?;
    request.host = endpoints.host.clone();
    request.port = endpoints.rdp_port;
    // `host`/`port` above are the local SSH-forward endpoint. Certificate
    // pinning must stay scoped to the real Jetson rather than all tunnels
    // sharing 127.0.0.1.
    request.certificate_name = Some(certificate_name);
    Ok(endpoints)
}

const FIRST_DESKTOP_TIMEOUT: Duration = Duration::from_secs(18);
const REPAIRED_DESKTOP_TIMEOUT: Duration = Duration::from_secs(25);
/// Grace for CLIPRDR's MonitorReady after the desktop is already usable. On a
/// healthy device the handshake completes during connect, so this costs nothing;
/// it only elapses when xrdp never reached its channel server (KI-045).
const CLIPBOARD_READY_GRACE: Duration = Duration::from_secs(3);

async fn wait_for_usable_desktop(
    sessions: &RdpSessionManager,
    session_id: &str,
    timeout: Duration,
) -> bool {
    let deadline = tokio::time::Instant::now() + timeout;
    loop {
        if sessions.has_usable_frame_keyed(session_id) {
            return true;
        }
        if !sessions.is_running_keyed(session_id) || tokio::time::Instant::now() >= deadline {
            return false;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

/// Wait (briefly) for CLIPRDR to negotiate. Returns false if the session died or
/// the grace expired — both mean the clipboard will not work for this connection.
async fn wait_for_clip_ready(
    sessions: &RdpSessionManager,
    session_id: &str,
    timeout: Duration,
) -> bool {
    let deadline = tokio::time::Instant::now() + timeout;
    loop {
        if sessions.clip_ready_keyed(session_id) {
            return true;
        }
        if !sessions.is_running_keyed(session_id) || tokio::time::Instant::now() >= deadline {
            return false;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

/// Restart the XRDP stack so the NEXT connect gets a working channel server
/// (KI-045). Ordering is the whole point: `xrdp.service` carries
/// `RuntimeDirectory=xrdp`, so restarting it recreates `/run/xrdp` and deletes
/// the listen socket (`/run/xrdp/sockdir/xrdp_chansrv_socket_<display>`) out
/// from under a still-running `xrdp-chansrv`. That process never recreates it —
/// chansrv only creates the socket at startup — so xrdp logs
/// `xrdp_mm_connect_chansrv: connect failed`, abandons the channel server for
/// that connection, and the clipboard stays dead until someone kills chansrv
/// (a plain reconnect never heals it). Hence: kill chansrv first, then restart.
/// The real password travels over SSH stdin.
async fn restart_remote_desktop_services(
    app: &AppHandle,
    endpoints: &TunnelEndpoints,
    identity_host: &str,
    identity_port: u16,
    username: &str,
    password: &str,
) -> Result<(), RdpIpcError> {
    let wire = SshConnectionInput {
        host: endpoints.host.clone(),
        port: endpoints.ssh_port,
        username: username.to_owned(),
        device_id: None,
        password: None,
    };
    let config_dir = app
        .path()
        .app_config_dir()
        .map_err(|_| map_rdp_error(RdpError::Unknown))?;
    let trust = TrustStoreFile::load(config_dir).map_err(|_| map_rdp_error(RdpError::Unknown))?;
    let expected = trust
        .get_fingerprint(identity_host, identity_port)
        .ok_or_else(|| map_rdp_error(RdpError::Unknown))?;
    let mut session = match ssh::connect_with_identity(
        &wire,
        identity_host,
        identity_port,
        Some(&expected),
        &SshConfig::default(),
    )
    .await
    .map_err(|_| map_rdp_error(RdpError::Unknown))?
    {
        ssh::SshConnectOutcome::Connected(session) => session,
        _ => return Err(map_rdp_error(RdpError::Unknown)),
    };
    session
        .authenticate_password(username, password)
        .await
        .map_err(|_| map_rdp_error(RdpError::Unknown))?;
    let input = format!("{password}\n");
    let result = session
        .exec_with_stdin(
            "sudo -S -p '' sh -c 'pkill -x xrdp-chansrv 2>/dev/null; \
rm -f /run/xrdp/sockdir/xrdp_chansrv_socket_* 2>/dev/null; \
systemctl restart xrdp-sesman xrdp; rc=$?; \
pkill -f \"^/usr/lib/xorg/Xorg :1[0-9]\" 2>/dev/null; exit $rc'",
            input.as_bytes(),
        )
        .await
        .map_err(|_| map_rdp_error(RdpError::Unknown))?;
    match result.exit_code {
        Some(0) | None => {
            eprintln!(
                "[jr-flow] XRDP services restarted (xrdp-chansrv cleared first) and stale sessions cleared after no-desktop timeout"
            );
            Ok(())
        }
        Some(_) => Err(map_rdp_error(RdpError::Unknown)),
    }
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
    focus_on_launch: Option<bool>,
    mut request: RdpConnectionRequest,
) -> Result<RdpLaunchResult, RdpIpcError> {
    if matches!(engine(), Engine::Sidecar) {
        // Dev-only sidecar engine predates multi-session; keep it single.
        return Err(map_rdp_error(RdpError::Unknown));
    }
    if session_id.is_empty() {
        return Err(map_rdp_error(RdpError::Unknown));
    }
    let launch_mode = if focus_on_launch.unwrap_or(true) {
        SessionLaunchMode::Focused
    } else {
        SessionLaunchMode::Background
    };
    if embedded.is_running_keyed(&session_id) && embedded.has_usable_frame_keyed(&session_id) {
        if launch_mode == SessionLaunchMode::Focused {
            let ns_window = window_ns_window(&app).map_err(map_rdp_error)?;
            embedded
                .focus(&session_id, ns_window, SESSION_TAB_BAR_INSET)
                .map_err(map_rdp_error)?;
        }
        return Ok(RdpLaunchResult::AlreadyRunning);
    }

    let device_id = request.device_id.clone();
    // Serialize launches for this key. Without this, two overlapping calls both
    // reach `sessions.insert` and the loser's session is dropped while its
    // worker is still running — the view is freed under a live `on_frame`
    // (KI-043 / audit N-01). Held for the rest of the command; a re-launch that
    // arrives meanwhile just gets `AlreadyRunning` and the 1 Hz status poll
    // settles the tab once the first launch lands.
    let _launch_claim = match embedded.claim_launch(&session_id) {
        Some(claim) => claim,
        None => {
            eprintln!("[jr-flow] rdp session launch already in flight, skipping id={session_id}");
            return Ok(RdpLaunchResult::AlreadyRunning);
        }
    };
    let endpoints = prepare_request(&app, &secrets, &tunnels, &mut request).await?;

    let config = RdpConnectionConfig::from(request);
    eprintln!(
        "[jr-flow] rdp session launch id={} host={} port={}",
        session_id, config.host, config.port
    );
    // Raw AppKit pointers are not `Send`; keep only the address across the
    // async readiness/repair waits and reconstruct the non-owning pointer for
    // each synchronous manager call.
    let ns_window_addr = window_ns_window(&app).map_err(map_rdp_error)? as usize;
    // A prior transport can still be alive while showing XRDP's permanent
    // white pre-session buffer. Replace it instead of returning
    // AlreadyRunning and perpetuating the false-positive connection state.
    if embedded.is_running_keyed(&session_id) {
        embedded
            .close_keyed(&session_id)
            .await
            .map_err(map_rdp_error)?;
    }
    embedded
        .launch_keyed(
            &session_id,
            &config,
            ns_window_addr as *mut c_void,
            SESSION_TAB_BAR_INSET,
            SessionLaunchMode::Background,
        )
        .map_err(map_rdp_error)?;
    let desktop_up = wait_for_usable_desktop(&embedded, &session_id, FIRST_DESKTOP_TIMEOUT).await;
    // A usable desktop only proves the *screen* works: CLIPRDR rides a separate
    // channel served by `xrdp-chansrv`, which xrdp can fail to reach while
    // everything else looks healthy. MonitorReady arrives during connect, so
    // this grace costs nothing unless the channel server is unreachable — in
    // which case we take the same repair path, which now clears chansrv first
    // (KI-045). One repair per launch: falling through happens at most once.
    let clipboard_up =
        desktop_up && wait_for_clip_ready(&embedded, &session_id, CLIPBOARD_READY_GRACE).await;
    if desktop_up && clipboard_up {
        if launch_mode == SessionLaunchMode::Focused {
            embedded
                .focus(
                    &session_id,
                    ns_window_addr as *mut c_void,
                    SESSION_TAB_BAR_INSET,
                )
                .map_err(map_rdp_error)?;
        }
        return Ok(RdpLaunchResult::Opened);
    }

    if desktop_up {
        eprintln!(
            "[jr-flow] clipboard channel never negotiated (xrdp could not reach xrdp-chansrv); repairing"
        );
    } else {
        eprintln!("[jr-flow] no usable desktop frame; repairing XRDP services");
    }
    embedded
        .close_keyed(&session_id)
        .await
        .map_err(map_rdp_error)?;
    let repair_identity = device_id
        .as_deref()
        .filter(|i| !i.is_empty())
        .unwrap_or(&config.certificate_name)
        .to_string();
    restart_remote_desktop_services(
        &app,
        &endpoints,
        &repair_identity,
        crate::ssh::types::DEFAULT_PORT,
        &config.username,
        &config.password,
    )
    .await?;
    tokio::time::sleep(Duration::from_secs(2)).await;

    embedded
        .launch_keyed(
            &session_id,
            &config,
            ns_window_addr as *mut c_void,
            SESSION_TAB_BAR_INSET,
            SessionLaunchMode::Background,
        )
        .map_err(map_rdp_error)?;
    if wait_for_usable_desktop(&embedded, &session_id, REPAIRED_DESKTOP_TIMEOUT).await {
        // Only a warning: this launch gets one repair, and a desktop without a
        // clipboard still beats no desktop at all.
        if !wait_for_clip_ready(&embedded, &session_id, CLIPBOARD_READY_GRACE).await {
            eprintln!(
                "[jr-flow] clipboard still unavailable after repair (CLIPRDR never negotiated); keeping the desktop"
            );
        }
        if launch_mode == SessionLaunchMode::Focused {
            embedded
                .focus(
                    &session_id,
                    ns_window_addr as *mut c_void,
                    SESSION_TAB_BAR_INSET,
                )
                .map_err(map_rdp_error)?;
        }
        return Ok(RdpLaunchResult::Opened);
    }

    embedded
        .close_keyed(&session_id)
        .await
        .map_err(map_rdp_error)?;
    Err(map_rdp_error(RdpError::NoUsableFrame))
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
    eprintln!("[jr-flow] close_session id={session_id}");
    match engine() {
        Engine::Embedded => embedded
            .close_keyed(&session_id)
            .await
            .map_err(map_rdp_error),
        Engine::Sidecar => Err(map_rdp_error(RdpError::Unknown)),
    }
}

/// One session's status (frontend polls per active tab). `(async)` — see
/// `rdp_status` for why the main thread must never take the manager lock.
#[tauri::command(async)]
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
/// detection for all connected devices). `(async)` — see `rdp_status`; this one
/// is polled every second, so it was the most reliable trigger of the freeze.
#[tauri::command(async)]
pub fn all_session_statuses(embedded: State<'_, RdpSessionManager>) -> Vec<SessionStatusEntry> {
    embedded
        .all_statuses()
        .into_iter()
        .map(|(session_id, status)| SessionStatusEntry { session_id, status })
        .collect()
}

fn map_tunnel_rdp_error(e: TunnelError) -> RdpIpcError {
    use crate::rdp::error::RdpErrorCode;
    let code = e.code();
    let (code, message) = match e {
        TunnelError::ExternalSingleDevice => (
            RdpErrorCode::RdpUnknown,
            format!("{code}: single-device external tunnel"),
        ),
        TunnelError::AuthFailed => (
            RdpErrorCode::RdpAuthenticationFailed,
            format!("{code}: Authentication failed"),
        ),
        TunnelError::Unreachable => (
            RdpErrorCode::RdpConnectionFailed,
            format!("{code}: Could not reach the device"),
        ),
        TunnelError::LocalPort(m) => (RdpErrorCode::RdpUnknown, format!("{code}: {m}")),
        TunnelError::SshExited(m) => (RdpErrorCode::RdpUnknown, format!("{code}: {m}")),
        TunnelError::Setup(m) => (RdpErrorCode::RdpUnknown, format!("{code}: {m}")),
    };
    RdpIpcError { code, message }
}
