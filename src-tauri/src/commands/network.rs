use std::time::Duration;

use crate::net::probe::TcpProbe;

const PROBE_TIMEOUT: Duration = Duration::from_secs(5);

/// Raw TCP reachability probe run inside the app process (same identity as the
/// app). Returns the OS errno verbatim — NOT mapped to a product error code.
#[tauri::command]
pub fn network_probe(host: String, port: u16) -> TcpProbe {
    crate::net::probe::tcp_probe(&host, port, PROBE_TIMEOUT)
}
