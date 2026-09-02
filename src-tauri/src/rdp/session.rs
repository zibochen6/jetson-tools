//! Embedded RDP session: owns the C bridge session (worker thread) + the native
//! view, routing frames from the FreeRDP worker thread to the view's layer.

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

/// Match the RDP desktop to the window's content aspect so the native view
/// blits without letterboxing and pointer mapping stays exact (scale ~1:1 for
/// a landscape window). Session size is fixed at login; a reconnect picks up
/// the new geometry.
fn desktop_size_for(ns_window: *mut c_void) -> (c_int_raw, c_int_raw) {
    let (mut w, mut h) = (0.0f64, 0.0f64);
    unsafe { ffi::jr_window_content_size(ns_window, &mut w, &mut h) };
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
}

// SAFETY: the C bridge and native view dispatch to their own thread / the main
// thread; the raw pointers are confined here and never shared unsafely.
unsafe impl Send for RdpSession {}

impl RdpSession {
    /// `ns_window` is a non-owning handle to the live Tauri main window (valid
    /// for the app lifetime); the `.m` layer confines AppKit use to the main
    /// thread, so no caller `unsafe` is required.
    #[allow(clippy::not_unsafe_ptr_arg_deref)]
    pub fn launch(config: &RdpConnectionConfig, ns_window: *mut c_void) -> Result<Self, RdpError> {
        let view = NativeView::create();
        view.set_fill(0x1e, 0x20, 0x26); // subtle dark placeholder until first frame
                                         // SAFETY: `ns_window` is the live Tauri main window handle.
        unsafe { view.add_to_window(ns_window) };

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

        let (dw, dh) = desktop_size_for(ns_window);
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
        })
    }

    pub fn is_running(&self) -> bool {
        !self.finished.load(Ordering::SeqCst)
    }

    /// Disconnect + join the worker + destroy the C session + reclaim the ctx.
    pub fn shutdown(&mut self) {
        if self.session.is_null() {
            return;
        }
        // Stop input forwarding first (synchronous), so no event handler can
        // touch the session after `jr_session_destroy` below.
        self._view.attach_input(None);
        unsafe { ffi::jr_clipboard_sync_stop() };
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

/// Single active embedded session (MVP: one device at a time), managed state.
pub struct RdpSessionManager {
    active: Mutex<Option<RdpSession>>,
}

impl Default for RdpSessionManager {
    fn default() -> Self {
        Self::new()
    }
}

impl RdpSessionManager {
    pub fn new() -> Self {
        Self {
            active: Mutex::new(None),
        }
    }

    pub fn is_running(&self) -> bool {
        self.active
            .lock()
            .unwrap()
            .as_ref()
            .is_some_and(|s| s.is_running())
    }

    #[allow(clippy::not_unsafe_ptr_arg_deref)]
    pub fn launch(
        &self,
        config: &RdpConnectionConfig,
        ns_window: *mut c_void,
    ) -> Result<(), RdpError> {
        let session = RdpSession::launch(config, ns_window)?;
        let mut guard = self.active.lock().unwrap();
        // Re-launch (retry) path: clean up any prior exited session — its worker
        // already finished; `shutdown` frees the C session + context box.
        if let Some(mut old) = guard.take() {
            old.shutdown();
        }
        *guard = Some(session);
        Ok(())
    }

    pub fn status(&self) -> RdpStatus {
        let mut guard = self.active.lock().unwrap();
        match guard.as_mut() {
            None => RdpStatus::NotRunning,
            Some(s) if s.is_running() => RdpStatus::Running,
            Some(s) => RdpStatus::Exited {
                exit_code: None,
                error: s.exit_error.lock().unwrap().clone(),
            },
        }
    }

    /// Take the session out so the graceful shutdown (which blocks on join) runs
    /// without holding the lock.
    pub async fn close(&self) -> Result<(), RdpError> {
        let session = self.active.lock().unwrap().take();
        if let Some(mut s) = session {
            s.shutdown();
        }
        Ok(())
    }

    pub fn clear_if_exited(&self) {
        let mut guard = self.active.lock().unwrap();
        if let Some(s) = guard.as_mut() {
            if !s.is_running() {
                *guard = None;
            }
        }
    }
}
