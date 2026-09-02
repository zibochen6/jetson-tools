//! FFI declarations for the native FreeRDP bridge (`native/freerdp_bridge`).
//!
//! Raw pointers and `unsafe` are confined here and in `session.rs`; they never
//! leak into Tauri commands, frontend DTOs, or the rest of the app.

use std::ffi::{c_char, c_void};
use std::os::raw::{c_double, c_int};

/// Opaque session handle (defined in `bridge.c`) — only ever used behind a
/// pointer, so a zero-sized `#[repr(C)]` body makes it FFI-safe.
#[allow(non_camel_case_types)]
#[repr(C)]
pub struct jr_session {
    _private: [u8; 0],
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct jr_session_callbacks {
    pub user: *mut c_void,
    pub on_connected: Option<unsafe extern "C" fn(*mut c_void)>,
    pub on_disconnected: Option<unsafe extern "C" fn(*mut c_void)>,
    pub on_frame_updated: Option<unsafe extern "C" fn(*mut c_void, c_int, c_int, c_int, c_int)>,
    pub on_desktop_resized: Option<unsafe extern "C" fn(*mut c_void, c_int, c_int)>,
    pub on_log: Option<unsafe extern "C" fn(*mut c_void, *const c_char)>,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct jr_cert_info {
    pub host: *const c_char,
    pub common_name: *const c_char,
    pub subject: *const c_char,
    pub issuer: *const c_char,
    pub fingerprint: *const c_char,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct jr_cert_callbacks {
    pub user: *mut c_void,
    pub verify_certificate: Option<unsafe extern "C" fn(*mut c_void, *const jr_cert_info) -> c_int>,
    pub verify_changed_certificate: Option<
        unsafe extern "C" fn(*mut c_void, *const jr_cert_info, *const jr_cert_info) -> c_int,
    >,
}

#[repr(C)]
pub struct jr_connect_params {
    pub host: *const c_char,
    pub port: u16,
    pub username: *const c_char,
    pub password: *const c_char,
    pub width: c_int,
    pub height: c_int,
    pub color_depth: c_int,
}

extern "C" {
    pub fn jr_freerdp_version() -> *const c_char;
    pub fn jr_session_create(
        params: *const jr_connect_params,
        cb: *const jr_session_callbacks,
        cert: *const jr_cert_callbacks,
    ) -> *mut jr_session;
    pub fn jr_session_destroy(s: *mut jr_session);
    pub fn jr_session_connect(s: *mut jr_session) -> c_int;
    pub fn jr_session_disconnect(s: *mut jr_session) -> c_int;
    pub fn jr_session_get_size(s: *mut jr_session, w: *mut c_int, h: *mut c_int) -> c_int;
    pub fn jr_session_get_framebuffer(
        s: *mut jr_session,
        buffer: *mut *const u8,
        w: *mut c_int,
        h: *mut c_int,
        stride: *mut c_int,
    ) -> c_int;
    pub fn jr_session_send_mouse_move(s: *mut jr_session, x: c_int, y: c_int, buttons: c_int) -> c_int;
    pub fn jr_session_send_mouse_button(
        s: *mut jr_session,
        button: c_int,
        down: c_int,
        x: c_int,
        y: c_int,
    ) -> c_int;
    pub fn jr_session_send_mouse_wheel(
        s: *mut jr_session,
        delta: c_int,
        negative: c_int,
        hdelta: c_int,
        hnegative: c_int,
        x: c_int,
        y: c_int,
    ) -> c_int;
    pub fn jr_session_send_key_scancode(
        s: *mut jr_session,
        down: c_int,
        repeat: c_int,
        scancode: c_int,
        extended: c_int,
    ) -> c_int;
    pub fn jr_session_set_size(s: *mut jr_session, w: c_int, h: c_int) -> c_int;
    pub fn jr_last_error(s: *mut jr_session) -> *const c_char;

    // Native desktop view (macos_view.m).
    pub fn jr_view_create() -> *mut c_void;
    pub fn jr_view_destroy(view: *mut c_void);
    pub fn jr_view_set_frame(view: *mut c_void, x: c_double, y: c_double, w: c_double, h: c_double);
    pub fn jr_view_add_to_window(view: *mut c_void, ns_window: *mut c_void);
    pub fn jr_view_remove_from_window(view: *mut c_void);
    pub fn jr_view_set_fill(view: *mut c_void, r: u8, g: u8, b: u8);
    pub fn jr_view_attach_input(view: *mut c_void, session: *mut c_void);
    pub fn jr_window_content_size(ns_window: *mut c_void, w: *mut c_double, h: *mut c_double);
    pub fn jr_session_set_clipboard_text(s: *mut jr_session, utf8: *const c_char) -> c_int;
    pub fn jr_clipboard_sync_start(session: *mut c_void);
    pub fn jr_clipboard_sync_stop();
    pub fn jr_view_present_buffer(
        view: *mut c_void,
        buffer: *const u8,
        w: c_int,
        h: c_int,
        stride: c_int,
        dx: c_int,
        dy: c_int,
        dw: c_int,
        dh: c_int,
    );
}
