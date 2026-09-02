//! Embedded RDP session: owns the C bridge session (worker thread) + the native
//! view, routing frames from the FreeRDP worker thread to the view's layer.
//!
//! V0.4: the manager holds ONE SESSION PER DEVICE (keyed) so several Jetsons
//! can stay connected at once. Only the *focused* session's view is mounted in
//! the window (below the webview tab-bar strip) and receives input + clipboard
//! sync; background sessions keep running headless and blit into their own
//! detached layers.

use std::collections::HashMap;
use std::ffi::{c_int, c_void, CStr, CString};
use std::os::raw::c_int as c_int_raw;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;

use super::error::RdpError;
use super::ffi;
use super::native_view::NativeView;
use super::types::{RdpConnectionConfig, RdpStatus};

const DEFAULT_WIDTH: c_int_raw = 1280;
const DEFAULT_HEIGHT: c_int_raw = 720;

/// Reserved key for the legacy single-session commands (`launch_remote_desktop`
/// et al.), which predate the keyed multi-device API (V0.4).
pub const LEGACY_SESSION_KEY: &str = "__legacy__";

/// Match the RDP desktop to the window's content aspect so the native view
/// blits without letterboxing and pointer mapping stays exact (scale ~1:1 for
/// a landscape window). Session size is fixed at login; a reconnect picks up
/// the new geometry. `top_inset` reserves the tab-bar strip (V0.4) so the
/// session aspect matches the visible surface.
fn desktop_size_for(ns_window: *mut c_void, top_inset: f64) -> (c_int_raw, c_int_raw) {
    let (mut w, mut h) = (0.0f64, 0.0f64);
    unsafe { ffi::jr_window_content_size(ns_window, &mut w, &mut h) };
    h = (h - top_inset).max(100.0);
    if w < 100.0 || h < 100.0 {
        return (DEFAULT_WIDTH, DEFAULT_HEIGHT);
    }
    let dw = (w.round() as c_int_raw).clamp(1024, 2560);
    let dh = ((dw as f64) * h / w).round().clamp(600.0, 2048.0) as c_int_raw;
    (dw, dh)
}

/// Shared state read by the C callbacks via the `user` pointer.
struct SessionContext {
    session: *mut ffi::jr_session,
    view: *mut c_void,
}

unsafe extern "C" fn on_frame(
    user: *mut c_void,
    _x: c_int_raw,
    _y: c_int_raw,
    _w: c_int_raw,
    _h: c_int_raw,
) {
    // SAFETY: `user` is a boxed `SessionContext` that outlives the worker thread
    // (reclaimed only after the worker is joined, in `RdpSession::shutdown`).
    let ctx = unsafe { &*(user as *const SessionContext) };
    let mut buf: *const u8 = std::ptr::null();
    let mut w: c_int_raw = 0;
    let mut h: c_int_raw = 0;
    let mut stride: c_int_raw = 0;
    let rc = unsafe {
        ffi::jr_session_get_framebuffer(ctx.session, &mut buf, &mut w, &mut h, &mut stride)
    };
    if rc == 0 && !buf.is_null() {
        // Copies the buffer before returning (macos_view.m does the copy).
        unsafe { ffi::jr_view_present_buffer(ctx.view, buf, w, h, stride, 0, 0, w, h) };
    }
}

unsafe extern "C" fn noop(_user: *mut c_void) {}

unsafe extern "C" fn verify_cert(_user: *mut c_void, _info: *const ffi::jr_cert_info) -> c_int {
    1 // TOFU: accept + store on first sight (mirrors `/cert:tofu`).
}

unsafe extern "C" fn verify_changed(
    _user: *mut c_void,
    _new: *const ffi::jr_cert_info,
    _old: *const ffi::jr_cert_info,
) -> c_int {
    0 // changed certificate → reject (never auto-accept).
}

pub struct RdpSession {
    session: *mut ffi::jr_session,
    _view: NativeView,
    context: *mut SessionContext,
    worker: Option<JoinHandle<c_int>>,
    finished: Arc<AtomicBool>,
    /// Last C-bridge error string (empty on clean close), surfaced via status.
    exit_error: Arc<Mutex<Option<String>>>,
    /// This session currently owns the global clipboard sync (only one can).
    clipboard_active: bool,
    /// This session's view currently forwards input (only the mounted one does).
    input_attached: bool,
}

// SAFETY: the C bridge and native view dispatch to their own thread / the main
// thread; the raw pointers are confined here and never shared unsafely.
unsafe impl Send for RdpSession {}

impl RdpSession {
    /// Legacy entry point: mount filling the whole window (no tab-bar inset).
    /// `ns_window` is a non-owning handle to the live Tauri main window (valid
    /// for the app lifetime); the `.m` layer confines AppKit use to the main
    /// thread, so no caller `unsafe` is required.
    #[allow(clippy::not_unsafe_ptr_arg_deref)]
    pub fn launch(config: &RdpConnectionConfig, ns_window: *mut c_void) -> Result<Self, RdpError> {
        Self::launch_inner(config, ns_window, None)
    }

    /// Multi-device entry point (V0.4): mount below a `top_inset`-point strip
    /// reserved for the webview tab bar.
    #[allow(clippy::not_unsafe_ptr_arg_deref)]
    pub fn launch_inset(
        config: &RdpConnectionConfig,
        ns_window: *mut c_void,
        top_inset: f64,
    ) -> Result<Self, RdpError> {
        Self::launch_inner(config, ns_window, Some(top_inset))
    }

    fn launch_inner(
        config: &RdpConnectionConfig,
        ns_window: *mut c_void,
        top_inset: Option<f64>,
    ) -> Result<Self, RdpError> {
        let view = NativeView::create();
        view.set_fill(0x1e, 0x20, 0x26); // subtle dark placeholder until first frame
        match top_inset {
            // SAFETY: `ns_window` is the live Tauri main window handle.
            Some(inset) => unsafe { view.add_to_window_inset(ns_window, inset) },
            None => unsafe { view.add_to_window(ns_window) },
        }

        let finished = Arc::new(AtomicBool::new(false));
        let ctx = Box::new(SessionContext {
            session: std::ptr::null_mut(),
            view: view.raw(),
        });
        // SAFETY: the box is reclaimed in `shutdown` after the worker is joined.
        let ctx_ptr: *mut SessionContext = Box::into_raw(ctx);

        let host = CString::new(config.host.clone()).map_err(|_| RdpError::Unknown)?;
        let username = CString::new(config.username.clone()).map_err(|_| RdpError::Unknown)?;
        let password = CString::new(config.password.clone()).map_err(|_| RdpError::Unknown)?;

        let (dw, dh) = desktop_size_for(ns_window, top_inset.unwrap_or(0.0));
        let params = ffi::jr_connect_params {
            host: host.as_ptr(),
            port: config.port,
            username: username.as_ptr(),
            password: password.as_ptr(),
            width: dw,
            height: dh,
            color_depth: 32,
        };
        let cb = ffi::jr_session_callbacks {
            user: ctx_ptr as *mut c_void,
            on_connected: Some(noop),
            on_disconnected: Some(noop),
            on_frame_updated: Some(on_frame),
            on_desktop_resized: None,
            on_log: None,
        };
        let cert = ffi::jr_cert_callbacks {
            user: ctx_ptr as *mut c_void,
            verify_certificate: Some(verify_cert),
            verify_changed_certificate: Some(verify_changed),
        };

        let session = unsafe { ffi::jr_session_create(&params, &cb, &cert) };
        if session.is_null() {
            // SAFETY: reclaiming the box we just leaked.
            unsafe { drop(Box::from_raw(ctx_ptr)) };
            return Err(RdpError::Unknown);
        }
        // SAFETY: worker joins before this is reclaimed.
        unsafe { (*ctx_ptr).session = session };
        // Forward AppKit input events from the view into this session. Must be
        // detached (synchronously) before `jr_session_destroy` in `shutdown`.
        view.attach_input(Some(session));
        // Mac pasteboard <-> remote clipboard (CLIPRDR) text sync.
        unsafe { ffi::jr_clipboard_sync_start(session as *mut c_void) };

        let saddr = session as usize;
        let finished2 = finished.clone();
        let exit_error: Arc<Mutex<Option<String>>> = Arc::new(Mutex::new(None));
        let exit_error2 = exit_error.clone();
        let worker = std::thread::spawn(move || {
            let rc = unsafe { ffi::jr_session_connect(saddr as *mut ffi::jr_session) };
            // Capture the bridge's last error before the session is destroyed in
            // `shutdown` (after join); empty means a clean disconnect.
            let err = unsafe { CStr::from_ptr(ffi::jr_last_error(saddr as *mut ffi::jr_session)) }
                .to_string_lossy()
                .into_owned();
            if !err.is_empty() {
                *exit_error2.lock().unwrap() = Some(err);
            }
            finished2.store(true, Ordering::SeqCst);
            rc
        });

        Ok(Self {
            session,
            _view: view,
            context: ctx_ptr,
            worker: Some(worker),
            finished,
            exit_error,
            clipboard_active: true,
            input_attached: true,
        })
    }

    pub fn is_running(&self) -> bool {
        !self.finished.load(Ordering::SeqCst)
    }

    /// Yield the screen: detach input + clipboard, remove the view from the
    /// window. The RDP connection itself keeps running (multi-device, V0.4).
    pub fn unfocus(&mut self) {
        if self.session.is_null() {
            return;
        }
        if self.input_attached {
            self._view.attach_input(None);
            self.input_attached = false;
        }
        if self.clipboard_active {
            unsafe { ffi::jr_clipboard_sync_stop() };
            self.clipboard_active = false;
        }
        self._view.remove_from_window();
    }

    /// Take the screen back: remount the view below the tab-bar strip and
    /// re-attach input + clipboard.
    #[allow(clippy::not_unsafe_ptr_arg_deref)]
    pub fn refocus(&mut self, ns_window: *mut c_void, top_inset: f64) {
        if self.session.is_null() {
            return;
        }
        // SAFETY: `ns_window` is the live Tauri main window handle.
        unsafe { self._view.add_to_window_inset(ns_window, top_inset) };
        self._view.attach_input(Some(self.session));
        self.input_attached = true;
        if !self.clipboard_active {
            unsafe { ffi::jr_clipboard_sync_start(self.session as *mut c_void) };
            self.clipboard_active = true;
        }
    }

    /// Disconnect + join the worker + destroy the C session + reclaim the ctx.
    pub fn shutdown(&mut self) {
        if self.session.is_null() {
            return;
        }
        // Stop input forwarding first (synchronous), so no event handler can
        // touch the session after `jr_session_destroy` below.
        if self.input_attached {
            self._view.attach_input(None);
            self.input_attached = false;
        }
        if self.clipboard_active {
            unsafe { ffi::jr_clipboard_sync_stop() };
            self.clipboard_active = false;
        }
        unsafe { ffi::jr_session_disconnect(self.session) };
        if let Some(w) = self.worker.take() {
            let _ = w.join();
        }
        unsafe { ffi::jr_session_destroy(self.session) };
        self.session = std::ptr::null_mut();
        // SAFETY: worker joined → no callbacks in flight; reclaim the box.
        unsafe { drop(Box::from_raw(self.context)) };
        self.context = std::ptr::null_mut();
    }
}

/// Keyed embedded sessions (V0.4 multi-device). One session per key (usually
/// `username@host`); only the *focused* key is on screen. The legacy
/// single-session API delegates to the reserved [`LEGACY_SESSION_KEY`].
///
/// Lock order is always `sessions` → `focused` (never reversed).
pub struct RdpSessionManager {
    sessions: Mutex<HashMap<String, RdpSession>>,
    focused: Mutex<Option<String>>,
}

impl Default for RdpSessionManager {
    fn default() -> Self {
        Self::new()
    }
}

impl RdpSessionManager {
    pub fn new() -> Self {
        Self {
            sessions: Mutex::new(HashMap::new()),
            focused: Mutex::new(None),
        }
    }

    // ---- legacy single-session API (unchanged semantics) ----

    /// Legacy: any live session occupies the single desktop slot.
    pub fn is_running(&self) -> bool {
        self.any_running()
    }

    /// Legacy full-window launch under the reserved key (replaces any prior
    /// legacy session, mirroring the pre-V0.4 manager).
    #[allow(clippy::not_unsafe_ptr_arg_deref)]
    pub fn launch(
        &self,
        config: &RdpConnectionConfig,
        ns_window: *mut c_void,
    ) -> Result<(), RdpError> {
        self.launch_entry(LEGACY_SESSION_KEY, config, ns_window, None)
    }

    pub fn status(&self) -> RdpStatus {
        self.status_keyed(LEGACY_SESSION_KEY)
    }

    /// Take the session out so the graceful shutdown (which blocks on join) runs
    /// without holding the lock.
    pub async fn close(&self) -> Result<(), RdpError> {
        self.close_keyed(LEGACY_SESSION_KEY).await
    }

    pub fn clear_if_exited(&self) {
        let dead = {
            let mut sessions = self.sessions.lock().unwrap();
            sessions
                .get_mut(LEGACY_SESSION_KEY)
                .is_some_and(|s| !s.is_running())
        };
        if dead {
            let session = self.sessions.lock().unwrap().remove(LEGACY_SESSION_KEY);
            if let Some(mut s) = session {
                s.shutdown();
            }
        }
    }

    // ---- keyed multi-device API (V0.4) ----

    /// Launch a session under `id`, mounted below the tab-bar strip, and focus
    /// it (unfocusing whichever session was on screen). Re-launching an id
    /// shuts down its previous session first (retry path).
    #[allow(clippy::not_unsafe_ptr_arg_deref)]
    pub fn launch_keyed(
        &self,
        id: &str,
        config: &RdpConnectionConfig,
        ns_window: *mut c_void,
        top_inset: f64,
    ) -> Result<(), RdpError> {
        self.launch_entry(id, config, ns_window, Some(top_inset))
    }

    fn launch_entry(
        &self,
        id: &str,
        config: &RdpConnectionConfig,
        ns_window: *mut c_void,
        top_inset: Option<f64>,
    ) -> Result<(), RdpError> {
        // BEFORE creating the new session, make every prior session yield the
        // global resources (clipboard sync, input, screen). Ordering matters:
        // the outgoing sessions' `jr_clipboard_sync_stop` must run BEFORE the
        // successor starts its own sync, or it would kill the successor's.
        let prior_under_key = {
            let mut sessions = self.sessions.lock().unwrap();
            let mut focused = self.focused.lock().unwrap();
            if let Some(fid) = focused.as_deref() {
                if fid != id {
                    if let Some(prev) = sessions.get_mut(fid) {
                        prev.unfocus(); // keeps running headless
                    }
                }
            }
            *focused = None;
            sessions.remove(id) // re-launch (retry) path: same-key predecessor
        };
        if let Some(mut old) = prior_under_key {
            old.shutdown(); // blocking join — outside the lock on purpose
        }

        // On launch failure the screen stays empty (focused = None); the
        // frontend surfaces the error and can retry.
        let session = match top_inset {
            Some(inset) => RdpSession::launch_inset(config, ns_window, inset)?,
            None => RdpSession::launch(config, ns_window)?,
        };
        let mut sessions = self.sessions.lock().unwrap();
        sessions.insert(id.to_string(), session);
        *self.focused.lock().unwrap() = Some(id.to_string());
        Ok(())
    }

    /// Bring `id` to the screen without reconnecting (the tab-bar quick
    /// switch). Errors when the session is missing or already exited.
    #[allow(clippy::not_unsafe_ptr_arg_deref)]
    pub fn focus(&self, id: &str, ns_window: *mut c_void, top_inset: f64) -> Result<(), RdpError> {
        let mut sessions = self.sessions.lock().unwrap();
        let mut focused = self.focused.lock().unwrap();
        if focused.as_deref() == Some(id) {
            return Ok(()); // already on screen
        }
        let running = sessions.get_mut(id).is_some_and(|s| s.is_running());
        if !running {
            return Err(RdpError::Unknown);
        }
        if let Some(fid) = focused.take() {
            if let Some(prev) = sessions.get_mut(&fid) {
                prev.unfocus();
            }
        }
        if let Some(target) = sessions.get_mut(id) {
            target.refocus(ns_window, top_inset);
        }
        *focused = Some(id.to_string());
        Ok(())
    }

    /// Remove every session from the screen (show the webview home) while
    /// keeping the connections alive.
    pub fn hide_all(&self) {
        let mut sessions = self.sessions.lock().unwrap();
        let mut focused = self.focused.lock().unwrap();
        if let Some(fid) = focused.take() {
            if let Some(s) = sessions.get_mut(&fid) {
                s.unfocus();
            }
        }
    }

    /// Close ONE session, releasing its key. Idempotent; the remote
    /// Xorg/XFCE session is left alive (PRD §24).
    pub async fn close_keyed(&self, id: &str) -> Result<(), RdpError> {
        let session = {
            let mut sessions = self.sessions.lock().unwrap();
            let mut focused = self.focused.lock().unwrap();
            if focused.as_deref() == Some(id) {
                *focused = None;
            }
            sessions.remove(id)
        };
        if let Some(mut s) = session {
            s.shutdown();
        }
        Ok(())
    }

    pub fn status_keyed(&self, id: &str) -> RdpStatus {
        let mut sessions = self.sessions.lock().unwrap();
        match sessions.get_mut(id) {
            None => RdpStatus::NotRunning,
            Some(s) if s.is_running() => RdpStatus::Running,
            Some(s) => RdpStatus::Exited {
                exit_code: None,
                error: s.exit_error.lock().unwrap().clone(),
            },
        }
    }

    /// Snapshot of every session's status (frontend tab-bar polling).
    pub fn all_statuses(&self) -> Vec<(String, RdpStatus)> {
        let mut sessions = self.sessions.lock().unwrap();
        sessions
            .iter_mut()
            .map(|(id, s)| {
                let status = if s.is_running() {
                    RdpStatus::Running
                } else {
                    RdpStatus::Exited {
                        exit_code: None,
                        error: s.exit_error.lock().unwrap().clone(),
                    }
                };
                (id.clone(), status)
            })
            .collect()
    }

    pub fn is_running_keyed(&self, id: &str) -> bool {
        self.sessions
            .lock()
            .unwrap()
            .get_mut(id)
            .is_some_and(|s| s.is_running())
    }

    pub fn any_running(&self) -> bool {
        self.sessions
            .lock()
            .unwrap()
            .values_mut()
            .any(|s| s.is_running())
    }

    /// Release exited sessions (except the focused one — the frontend decides
    /// how to surface a dead desktop before it is swept).
    pub fn clear_exited(&self) {
        let mut sessions = self.sessions.lock().unwrap();
        let focused = self.focused.lock().unwrap().clone();
        let dead: Vec<String> = sessions
            .iter_mut()
            .filter(|(id, s)| Some(id.as_str()) != focused.as_deref() && !s.is_running())
            .map(|(id, _)| id.clone())
            .collect();
        for id in dead {
            if let Some(mut s) = sessions.remove(&id) {
                s.shutdown();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // NOTE: no test constructs an `RdpSession` directly — `NativeView::create`
    // dispatches to the AppKit main queue, which would deadlock under
    // `#[tokio::test]` worker threads. Manager bookkeeping that doesn't need a
    // live session is covered below; end-to-end paths ride the regression
    // guide (docs/CONNECTION_REGRESSION_GUIDE.md).

    #[test]
    fn empty_manager_reports_not_running() {
        let mgr = RdpSessionManager::new();
        assert!(!mgr.any_running());
        assert!(!mgr.is_running());
        assert_eq!(mgr.status_keyed("a"), RdpStatus::NotRunning);
        assert_eq!(mgr.status(), RdpStatus::NotRunning);
        assert!(mgr.all_statuses().is_empty());
    }

    #[tokio::test]
    async fn close_keyed_is_idempotent() {
        let mgr = RdpSessionManager::new();
        mgr.close_keyed("missing").await.unwrap();
        mgr.close_keyed("missing").await.unwrap();
        assert_eq!(mgr.status_keyed("missing"), RdpStatus::NotRunning);
    }

    #[test]
    fn hide_all_on_empty_manager_is_safe() {
        let mgr = RdpSessionManager::new();
        mgr.hide_all();
        assert!(!mgr.any_running());
    }

    #[test]
    fn clear_exited_on_empty_manager_is_safe() {
        let mgr = RdpSessionManager::new();
        mgr.clear_exited();
        assert!(mgr.all_statuses().is_empty());
    }
}
