pub mod bootstrap;
mod commands;
pub mod device;
/// Stable error codes shared with the frontend (Reliability Harness).
pub mod error_code;
/// Stable ID newtypes: SessionId / AttemptId / RunId / TransactionId.
pub mod ids;
pub mod net;
pub mod rdp;
pub mod remember;
/// Frozen RDP session contract layer (r2 §4): RdpEngine trait + SessionManager.
pub mod session;
pub mod ssh;
/// Shared test primitives (FakeClock / EventRecorder / FaultInjector) for
/// reliability scenario tests. Production code must never depend on this
/// module for behavior; the fault injector is release-neutral by design.
pub mod test_support;
pub mod trust;
pub mod tunnel;
pub mod updater;
pub mod updater_engine;

#[cfg(test)]
mod queue_ffi_tests;

#[cfg(test)]
mod clipboard_ffi_tests;

use tauri::Manager;

/// Platform log directory for the redirected stderr:
/// macOS `~/Library/Logs`, Linux `$XDG_STATE_HOME/jetson-remote`
/// (fallback `~/.local/state/jetson-remote`).
fn log_dir() -> Option<std::path::PathBuf> {
    #[cfg(target_os = "macos")]
    {
        let home = std::env::var("HOME").ok()?;
        Some(std::path::PathBuf::from(home).join("Library/Logs"))
    }
    #[cfg(target_os = "linux")]
    {
        if let Ok(state) = std::env::var("XDG_STATE_HOME") {
            if !state.is_empty() {
                return Some(std::path::PathBuf::from(state).join("jetson-remote"));
            }
        }
        let home = std::env::var("HOME").ok()?;
        Some(std::path::PathBuf::from(home).join(".local/state/jetson-remote"))
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    {
        None
    }
}

/// Release builds launched from Finder/`open` (or a Linux desktop launcher)
/// have no terminal: stderr (all `[jr-flow]` diagnostics) would vanish.
/// Redirect it to the platform log file so support can tail the flow
/// (CONNECTION_REGRESSION_GUIDE §4.2). Dev (`cargo tauri dev`) keeps the tty.
fn redirect_stderr_to_logfile() {
    use std::io::IsTerminal;
    use std::os::unix::io::AsRawFd;
    if std::io::stderr().is_terminal() {
        return;
    }
    let Some(dir) = log_dir() else {
        return;
    };
    let _ = std::fs::create_dir_all(&dir);
    let Ok(f) = std::fs::OpenOptions::new()
        .append(true)
        .create(true)
        .open(dir.join("jetson-remote.log"))
    else {
        return;
    };
    unsafe {
        libc::dup2(f.as_raw_fd(), 2);
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    redirect_stderr_to_logfile();
    // `run()` executes on the platform main thread (AppKit/GTK); the session
    // manager's lock-order assertions depend on knowing which thread that is
    // (P1-11). The GTK view also needs the identity explicitly: `.setup()`
    // runs between main-loop iterations, where context ownership is false
    // (KI-046).
    rdp::session::record_main_thread();
    #[cfg(target_os = "linux")]
    rdp::linux_view::record_main_thread();
    let app = tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .manage(rdp::manager::RdpProcessManager::new())
        .manage(rdp::session::RdpSessionManager::new())
        .manage(remember::FileSecretStore)
        .manage(tunnel::TunnelManager::new())
        .setup(|app| {
            #[cfg(not(target_os = "linux"))]
            let _ = app;
            // Linux: reparent the WebKitWebView into a GtkOverlay on the GTK
            // main thread so the embedded desktop can be stacked under the
            // HTML tab bar. Failure is logged and surfaces as `launch_session`
            // failing (no silent black window).
            #[cfg(target_os = "linux")]
            {
                use gtk::glib::translate::ToGlibPtr;
                use tauri::Manager;
                match app.get_webview_window("main") {
                    Some(win) => {
                        let window = win.gtk_window().ok().map(|w| {
                            let ptr: *mut gtk::ffi::GtkApplicationWindow = w.to_glib_none().0;
                            ptr.cast::<std::ffi::c_void>()
                        });
                        let vbox = win.default_vbox().ok().map(|v| {
                            let ptr: *mut gtk::ffi::GtkBox = v.to_glib_none().0;
                            ptr.cast::<std::ffi::c_void>()
                        });
                        match (window, vbox) {
                            (Some(window), Some(vbox)) => {
                                if let Err(err) = rdp::linux_view::init(window, vbox) {
                                    eprintln!("[jr-linux] embedded desktop init failed: {err}");
                                }
                            }
                            _ => eprintln!(
                                "[jr-linux] GTK window/vbox unavailable; embedded desktop disabled"
                            ),
                        }
                    }
                    None => eprintln!("[jr-linux] main window missing during setup"),
                }
            }
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::app::app_info,
            commands::app::health_check,
            commands::network::network_probe,
            commands::network::probe_device_paths,
            commands::connection::probe_device,
            commands::connection::prepare_remote_desktop,
            commands::rdp::launch_remote_desktop,
            commands::rdp::close_remote_desktop,
            commands::rdp::rdp_status,
            commands::rdp::launch_session,
            commands::rdp::focus_session,
            commands::rdp::close_session,
            commands::rdp::session_status,
            commands::rdp::all_session_statuses,
            commands::remember::get_remembered_devices,
            commands::remember::remember_device,
            commands::remember::forget_remembered_device,
            updater::check_for_update,
            updater::download_and_install_update,
        ])
        .build(tauri::generate_context!())
        .expect("error while building tauri application");

    app.run(|app_handle, event| {
        // Kill the loopback ssh tunnel (and remove its askpass secret files)
        // when the app quits; an orphaned `ssh -N` would outlive the UI.
        if let tauri::RunEvent::Exit = event {
            if let Some(tunnels) = app_handle.try_state::<tunnel::TunnelManager>() {
                tunnels.close_all();
            }
        }
    });
}
