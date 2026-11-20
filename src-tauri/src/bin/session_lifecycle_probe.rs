//! Headless verification for KI-043 — the `JRView` use-after-free (audit P1-5).
//!
//! Builds a real `RdpSession` against a real device *without* a Tauri window,
//! then tears it down through `RdpSession::shutdown`. With
//! `JR_RDP_STALL_AFTER_DISCONNECT_MS` set above the 4 s join budget, the worker
//! is still inside the FreeRDP event loop when the join deadline expires, so
//! teardown takes the abandon branch that used to free the view under a live
//! `on_frame`.
//!
//! Before the fix this binary dies with EXC_BAD_ACCESS inside `objc_retain`
//! (stack: `objc_retain ← jr_view_present_buffer ← on_frame`). After it, the run
//! completes and the diagnostic shows the view deliberately kept alive.
//!
//! Usage (from `src-tauri/`):
//! ```text
//! # start a tunnel first, e.g. ssh -N -L 127.0.0.1:13389:localhost:3389 seeed@<jetson>
//! JR_RDP_STALL_AFTER_DISCONNECT_MS=8000 printf '<password>\n' \
//!   | cargo run --bin session_lifecycle_probe -- 127.0.0.1 seeed 13389 <cert-name>
//! ```
//!
//! It runs on the process main thread, which AppKit treats as the main thread,
//! so `jr_view_create` takes its inline branch and needs no run loop.

use std::ffi::c_void;
use std::io::BufRead;
use std::time::Duration;

use jetson_remote_lib::rdp::ffi;
use jetson_remote_lib::rdp::session::{RdpSession, SessionLaunchMode};
use jetson_remote_lib::rdp::types::RdpConnectionConfig;

fn live_views() -> i32 {
    unsafe { ffi::jr_view_live_count() }
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let host = args.get(1).cloned().unwrap_or_else(|| "127.0.0.1".into());
    let username = args.get(2).cloned().unwrap_or_else(|| "seeed".into());
    let port: u16 = args.get(3).and_then(|v| v.parse().ok()).unwrap_or(3389);
    let certificate_name = args.get(4).cloned().unwrap_or_else(|| host.clone());

    let stall = std::env::var("JR_RDP_STALL_AFTER_DISCONNECT_MS").unwrap_or_else(|_| "0".into());
    println!(
        "[lifecycle] host={host} port={port} user={username} JR_RDP_STALL_AFTER_DISCONNECT_MS={stall}"
    );

    let mut password = String::new();
    if std::io::stdin().lock().read_line(&mut password).is_err() {
        eprintln!("[lifecycle] could not read password from stdin");
        std::process::exit(2);
    }

    let config = RdpConnectionConfig {
        certificate_name,
        host,
        port,
        username,
        password: password.trim().to_string(),
        dynamic_resolution: false,
        clipboard: false,
    };

    println!("[lifecycle] views before launch = {}", live_views());
    let mut session = match RdpSession::launch_inset(
        &config,
        std::ptr::null_mut::<c_void>(),
        0.0,
        SessionLaunchMode::Background,
    ) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("[lifecycle] launch failed: {e:?}");
            std::process::exit(3);
        }
    };
    println!("[lifecycle] launched; views = {}", live_views());

    // Let the worker connect and present a few frames, so `on_frame` is
    // demonstrably live at the moment we tear down.
    let mut had_frame = false;
    for i in 0..12 {
        std::thread::sleep(Duration::from_millis(1000));
        if session.has_usable_frame() {
            had_frame = true;
            println!("[lifecycle] t={}s usable frame observed", i + 1);
            break;
        }
    }
    if !had_frame {
        println!(
            "[lifecycle] WARNING: no usable frame before teardown — the device may be slow; \
                  the UAF window still requires a live worker, so a stall-only run proves less"
        );
    }

    // Teardown. With a stall longer than WORKER_JOIN_TIMEOUT this lands in the
    // abandon branch; the old code freed the view there and crashed on the next
    // frame, which arrives within one loop tick (~33 ms).
    println!("[lifecycle] shutting down (stall={stall}ms)…");
    session.shutdown();
    println!("[lifecycle] shutdown returned without crashing");

    // Give the abandoned worker time to resume and try to present. This is the
    // exact instant that used to fault.
    std::thread::sleep(Duration::from_millis(3000));
    let live = live_views();
    println!("[lifecycle] views after shutdown + 3s = {live}");

    // Drop the session struct — the other half of the ownership path.
    drop(session);
    std::thread::sleep(Duration::from_millis(1500));
    let live_after_drop = live_views();
    println!("[lifecycle] views after drop + 1.5s = {live_after_drop}");

    if stall != "0" && live_after_drop == 0 {
        println!(
            "[lifecycle] FAIL: the view was released while the abandoned worker could still \
             call on_frame — the UAF window is back"
        );
        std::process::exit(4);
    }
    println!("[lifecycle] OK");
}
