//! RAII wrapper over the ObjC native desktop view (`macos_view.m`).
//!
//! The `.m` layer dispatches every mutating call to the AppKit main thread, so
//! this handle is safe to share across threads (`unsafe impl Send`). The raw
//! pointer is confined here — it never leaks into the rest of the app.

use std::ffi::c_void;
use std::os::raw::c_int;

use super::ffi;

/// Owns a native `JRView` mounted as a subview of the Tauri window's
/// content view. Dropping it removes the view from the window and releases it.
pub struct NativeView {
    handle: *mut c_void,
}

// SAFETY: all `jr_view_*` calls below dispatch to the AppKit main thread
// internally, so the handle is thread-safe to store and call from any thread.
unsafe impl Send for NativeView {}

impl NativeView {
    pub fn create() -> Self {
        let handle = unsafe { ffi::jr_view_create() };
        debug_assert!(!handle.is_null(), "jr_view_create returned null");
        Self { handle }
    }

    pub fn raw(&self) -> *mut c_void {
        self.handle
    }

    /// Mount the view over the window's content view (fills it, autoresizing).
    ///
    /// # Safety
    /// `ns_window` must be a live `NSWindow` (the Tauri main window).
    pub unsafe fn add_to_window(&self, ns_window: *mut c_void) {
        unsafe { ffi::jr_view_add_to_window(self.handle, ns_window) };
    }

    pub fn remove_from_window(&self) {
        unsafe { ffi::jr_view_remove_from_window(self.handle) };
    }

    pub fn set_frame(&self, x: f64, y_top: f64, w: f64, h: f64) {
        unsafe { ffi::jr_view_set_frame(self.handle, x, y_top, w, h) };
    }

    /// Placeholder solid fill (spike / before the first frame).
    pub fn set_fill(&self, r: u8, g: u8, b: u8) {
        unsafe { ffi::jr_view_set_fill(self.handle, r, g, b) };
    }

    /// Forward AppKit mouse/keyboard events from this view into the session's
    /// RDP input channel. Synchronous; pass `None` to detach (must happen
    /// before the session is destroyed).
    pub fn attach_input(&self, session: Option<*mut ffi::jr_session>) {
        let ptr = session.map(|p| p as *mut c_void).unwrap_or(std::ptr::null_mut());
        unsafe { ffi::jr_view_attach_input(self.handle, ptr) };
    }

    /// Present a full framebuffer (copied by the `.m` before returning).
    ///
    /// # Safety
    /// `buffer` must point to `stride * h` bytes valid for the duration of the
    /// call (the FreeRDP framebuffer — valid only inside the frame callback).
    pub unsafe fn present(&self, buffer: *const u8, w: c_int, h: c_int, stride: c_int) {
        unsafe { ffi::jr_view_present_buffer(self.handle, buffer, w, h, stride, 0, 0, w, h) };
    }
}

impl Drop for NativeView {
    fn drop(&mut self) {
        unsafe { ffi::jr_view_destroy(self.handle) };
    }
}
