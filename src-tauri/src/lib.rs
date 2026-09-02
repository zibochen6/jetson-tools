pub mod bootstrap;
mod commands;
pub mod device;
pub mod net;
pub mod rdp;
pub mod remember;
pub mod ssh;
pub mod trust;
pub mod updater;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .manage(rdp::manager::RdpProcessManager::new())
        .manage(rdp::session::RdpSessionManager::new())
        .manage(remember::KeychainSecretStore::default())
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
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
