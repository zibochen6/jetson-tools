//! Linux desktop embedding bootstrap (GTK3).
//!
//! Tauri builds each window with a `GtkBox` (`default_vbox`) whose main child
//! is the `WebKitWebView`. [`init`] hands both raw widget pointers to
//! `native/freerdp_bridge/linux_view.c`, which reparents the webview into a
//! `GtkOverlay` inside that box so per-session `GtkDrawingArea`s can be stacked
//! above the page while the 44px HTML tab bar stays visible on top.
//!
//! All container/widget operations live in C; this module only marshals the
//! pointers (on the GTK main thread, from Tauri `.setup()`) and stores the
//! opaque overlay handle the session layer uses as its `ns_window`.

use std::ffi::c_void;
use std::sync::atomic::{AtomicUsize, Ordering};

use super::ffi;

static SURFACE: AtomicUsize = AtomicUsize::new(0);

/// Record the GTK main thread. Call once at the start of `run()` on the
/// process main thread: Tauri `.setup()` runs between main-loop iterations,
/// where `g_main_context_is_owner()` is false, so the C view needs an explicit
/// thread identity (KI-046).
pub fn record_main_thread() {
    unsafe { ffi::jr_linux_record_main_thread() };
}

/// Reparent the webview under a `GtkOverlay` and publish the surface handle.
///
/// Must run on the GTK main thread (Tauri `.setup()`); the C side enforces
/// this and returns an error otherwise.
pub fn init(window: *mut c_void, vbox: *mut c_void) -> Result<(), String> {
    if window.is_null() || vbox.is_null() {
        return Err("GTK window/vbox unavailable".to_string());
    }
    let rc = unsafe { ffi::jr_linux_init(window, vbox) };
    if rc != 0 {
        return Err(format!("jr_linux_init failed (code {rc})"));
    }
    let surface = unsafe { ffi::jr_linux_surface_handle() };
    if surface.is_null() {
        return Err("jr_linux_surface_handle returned null".to_string());
    }
    SURFACE.store(surface as usize, Ordering::Release);
    Ok(())
}

/// Opaque `GtkOverlay` pointer used as the `ns_window` handle by the session
/// layer. `None` until [`init`] succeeded (or if overlay setup failed).
pub fn surface_handle() -> Option<*mut c_void> {
    match SURFACE.load(Ordering::Acquire) {
        0 => None,
        raw => Some(raw as *mut c_void),
    }
}

#[cfg(test)]
pub fn reset_for_test() {
    SURFACE.store(0, Ordering::Release);
}
