//! Headless diagnostic: prove libfreerdp (via the C bridge) connects to a real
//! Jetson and receives a real framebuffer — no Tauri, no native view.
//!
//! Two independent signals are reported:
//!   1. `on_frame` / `EndPaint` firing (the legacy update path).
//!   2. direct sampling of `gdi->primary_buffer` — catches content delivered via
//!      the RDPGFX graphics pipeline even when `update->EndPaint` never fires.
//!
//! Usage: `printf 'PASSWORD\n' | cargo run --bin embedded_rdp_probe -- <host> [user] [port] [certificate-name]`

use std::ffi::{c_int, c_void, CStr, CString};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;
use std::thread;
use std::time::Duration;

use jetson_remote_lib::rdp::ffi::*;

struct Captured {
    frames: u32,
    width: c_int,
    height: c_int,
    stride: c_int,
    first: [u8; 8],
    has_frame: bool,
}

struct ProbeCtx {
    session: *mut jr_session,
    captured: Mutex<Captured>,
}

static ENDPAINT_COUNT: AtomicUsize = AtomicUsize::new(0);

unsafe extern "C" fn on_connected(_user: *mut c_void) {
    println!("[probe] connected");
}

unsafe extern "C" fn on_disconnected(_user: *mut c_void) {
    println!("[probe] disconnected");
}

unsafe extern "C" fn on_frame(user: *mut c_void, _x: c_int, _y: c_int, _w: c_int, _h: c_int) {
    ENDPAINT_COUNT.fetch_add(1, Ordering::SeqCst);
    let ctx = unsafe { &*(user as *const ProbeCtx) };
    let mut cap = ctx.captured.lock().unwrap();
    let mut buf: *const u8 = std::ptr::null();
    let mut w: c_int = 0;
    let mut h: c_int = 0;
    let mut stride: c_int = 0;
    let rc =
        unsafe { jr_session_get_framebuffer(ctx.session, &mut buf, &mut w, &mut h, &mut stride) };
    if rc == 0 && !buf.is_null() {
        cap.width = w;
        cap.height = h;
        cap.stride = stride;
        unsafe { std::ptr::copy_nonoverlapping(buf, cap.first.as_mut_ptr(), 8) };
        cap.has_frame = true;
    }
    cap.frames += 1;
}

unsafe extern "C" fn verify_cert(_user: *mut c_void, info: *const jr_cert_info) -> c_int {
    let fp = unsafe { CStr::from_ptr((*info).fingerprint) }.to_string_lossy();
    println!("[probe] certificate (TOFU: accept + store): fingerprint={fp}");
    1 // accept + store
}

unsafe extern "C" fn verify_changed(
    _user: *mut c_void,
    _new: *const jr_cert_info,
    _old: *const jr_cert_info,
) -> c_int {
    println!("[probe] certificate CHANGED — reject");
    0
}

/// Directly sample `gdi->primary_buffer` (decoupled from EndPaint). Prints a
/// compact line so content delivered via the GFX pipeline is still observable.
unsafe fn sample_buffer(session: *mut jr_session) {
    let mut buf: *const u8 = std::ptr::null();
    let mut w: c_int = 0;
    let mut h: c_int = 0;
    let mut stride: c_int = 0;
    let rc = unsafe { jr_session_get_framebuffer(session, &mut buf, &mut w, &mut h, &mut stride) };
    if rc != 0 || buf.is_null() || w <= 0 || h <= 0 {
        println!("[sample] no buffer yet");
        return;
    }
    let (w, h, stride) = (w as usize, h as usize, stride as usize);
    let n = 4096;
    let total = w * h;
    let nsamples = if total < n { total } else { n };
    let step = (total / nsamples).max(1);
    let mut nnz = 0usize;
    let mut minv = 255u8;
    let mut maxv = 0u8;
    for k in 0..nsamples {
        let idx = k * step;
        let x = idx % w;
        let y = idx / w;
        let p = unsafe { buf.add(y * stride + x * 4) };
        let b0 = unsafe { *p };
        let b1 = unsafe { *p.add(1) };
        let b2 = unsafe { *p.add(2) };
        let lum = b0.max(b1).max(b2);
        if b0 | b1 | b2 != 0 {
            nnz += 1;
        }
        if lum < minv {
            minv = lum;
        }
        if lum > maxv {
            maxv = lum;
        }
    }
    let c = unsafe { buf.add((h / 2) * stride + (w / 2) * 4) };
    let (c0, c1, c2, c3) = unsafe { (*c, *c.add(1), *c.add(2), *c.add(3)) };
    println!(
        "[sample] {}x{} stride={} nnz={}/{} min={} max={} center={:02X}{:02X}{:02X}{:02X}",
        w, h, stride, nnz, nsamples, minv, maxv, c0, c1, c2, c3
    );
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let host = args.get(1).map(String::as_str).unwrap_or("192.168.100.164");
    let username = args.get(2).map(String::as_str).unwrap_or("seeed");
    let port = args
        .get(3)
        .and_then(|value| value.parse().ok())
        .unwrap_or(3389);
    let certificate_name = args.get(4).map(String::as_str).unwrap_or(host);

    let mut password = String::new();
    if std::io::stdin().read_line(&mut password).is_err() {
        eprintln!("[probe] could not read password from stdin");
        std::process::exit(2);
    }
    let password = password.trim().to_string();

    let version = unsafe { CStr::from_ptr(jr_freerdp_version()) }.to_string_lossy();
    println!("[probe] FreeRDP bridge version: {version}");

    let host_c = CString::new(host).unwrap();
    let certificate_name_c = CString::new(certificate_name).unwrap();
    let user_c = CString::new(username).unwrap();
    let pass_c = CString::new(password).unwrap();

    let params = jr_connect_params {
        certificate_name: certificate_name_c.as_ptr(),
        host: host_c.as_ptr(),
        port,
        username: user_c.as_ptr(),
        password: pass_c.as_ptr(),
        width: 1280,
        height: 720,
        color_depth: 32,
    };

    let captured = Mutex::new(Captured {
        frames: 0,
        width: 0,
        height: 0,
        stride: 0,
        first: [0; 8],
        has_frame: false,
    });
    let boxed = Box::new(ProbeCtx {
        session: std::ptr::null_mut(),
        captured,
    });
    let ctx_ptr: *mut ProbeCtx = Box::into_raw(boxed);

    let cb = jr_session_callbacks {
        user: ctx_ptr as *mut c_void,
        on_connected: Some(on_connected),
        on_disconnected: Some(on_disconnected),
        on_frame_updated: Some(on_frame),
        on_desktop_resized: None,
        on_log: None,
    };
    let cert = jr_cert_callbacks {
        user: ctx_ptr as *mut c_void,
        verify_certificate: Some(verify_cert),
        verify_changed_certificate: Some(verify_changed),
    };

    let session = unsafe { jr_session_create(&params, &cb, &cert) };
    if session.is_null() {
        println!("[probe] jr_session_create FAILED");
        unsafe { jr_session_destroy(session) };
        let _ = unsafe { Box::from_raw(ctx_ptr) };
        std::process::exit(3);
    }
    unsafe { (*ctx_ptr).session = session };

    // Keep the session handle reachable for sampling while the connect thread
    // runs the blocking event loop; also drive a connect() from this thread.
    let session_addr = session as usize;
    let handle =
        thread::spawn(move || unsafe { jr_session_connect(session_addr as *mut jr_session) });

    // Poll ~24s: direct buffer sampling every ~2s + report EndPaint count.
    for i in 0..12 {
        thread::sleep(Duration::from_millis(2000));
        let ep = ENDPAINT_COUNT.load(Ordering::SeqCst);
        let fr = unsafe { (*ctx_ptr).captured.lock().unwrap().frames };
        println!("[probe] t={}s endpaint={} on_frame={}", i * 2, ep, fr);
        unsafe { sample_buffer(session_addr as *mut jr_session) };
    }

    unsafe { jr_session_disconnect(session) };
    let _ = handle.join();
    unsafe { jr_session_destroy(session) };
    let _ = unsafe { Box::from_raw(ctx_ptr) };
    println!("[probe] done");
}
