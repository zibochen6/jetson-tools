use serde::Serialize;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AppInfo {
    pub name: String,
    pub version: String,
}

/// Minimal IPC smoke boundary: proves React → invoke → Rust → React works.
/// Real SSH / provisioning / RDP commands arrive in later phases.
#[tauri::command]
pub fn app_info() -> AppInfo {
    AppInfo {
        name: "Jetson Remote".to_string(),
        version: env!("CARGO_PKG_VERSION").to_string(),
    }
}

#[tauri::command]
pub fn health_check() -> &'static str {
    "ok"
}
