pub mod bootstrap;
mod commands;
pub mod device;
pub mod net;
pub mod rdp;
pub mod remember;
pub mod ssh;
pub mod trust;
pub mod tunnel;
pub mod updater;

use tauri::Manager;

/// Release builds launched from Finder/`open` have no terminal: stderr (all
/// `[jr-flow]` diagnostics) would vanish. Redirect it to
/// `~/Library/Logs/jetson-remote.log` so support can tail the flow
/// (CONNECTION_REGRESSION_GUIDE §4.2). Dev (`cargo tauri dev`) keeps the tty.
fn redirect_stderr_to_logfile() {
    use std::io::IsTerminal;
    use std::os::unix::io::AsRawFd;
    if std::io::stderr().is_terminal() {
        return;
    }
    let Ok(home) = std::env::var("HOME") else {
        return;
    };
    let dir = std::path::PathBuf::from(home).join("Library/Logs");
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
    let app = tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .manage(rdp::manager::RdpProcessManager::new())
        .manage(rdp::session::RdpSessionManager::new())
        .manage(remember::FileSecretStore)
        .manage(tunnel::TunnelManager::new())
        .invoke_handler(tauri::generate_handler![
            commands::app::app_info,
            commands::app::health_check,
            commands::network::network_probe,
            commands::connection::probe_device,
            commands::connection::prepare_remote_desktop,
            commands::rdp::launch_remote_desktop,
            commands::rdp::close_remote_desktop,
            commands::rdp::rdp_status,
            commands::remember::get_remembered_device,
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
