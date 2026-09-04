use std::time::{Duration, Instant};

use serde::Serialize;
use tokio::net::TcpStream;

use crate::net::probe::TcpProbe;

const PROBE_TIMEOUT: Duration = Duration::from_secs(5);

/// Raw TCP reachability probe run inside the app process (same identity as the
/// app). Returns the OS errno verbatim — NOT mapped to a product error code.
#[tauri::command]
pub fn network_probe(host: String, port: u16) -> TcpProbe {
    crate::net::probe::tcp_probe(&host, port, PROBE_TIMEOUT)
}

/* ------------------------------------------------------------------ */
/* Multi-path device routing (identity-v3)                             */
/* ------------------------------------------------------------------ */

/// Per-address connect timeout for candidate-path probing. Short on purpose:
/// all candidates are probed in parallel, so this only bounds the wait for
/// the slowest unreachable address.
const PATH_PROBE_TIMEOUT: Duration = Duration::from_millis(800);

/// One row of `probe_device_paths`: raw TCP RTT to `address:22`.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PathProbeEntry {
    pub address: String,
    pub reachable: bool,
    pub rtt_ms: Option<u64>,
}

/// Probe every candidate address of a device in parallel (TCP `:22`) and
/// return reachability + RTT. The frontend orders the candidates by RTT and
/// tries them in sequence; unreachable entries are advisory, not errors.
#[tauri::command]
pub async fn probe_device_paths(addresses: Vec<String>) -> Vec<PathProbeEntry> {
    let tasks: Vec<_> = addresses
        .into_iter()
        .filter(|a| !a.trim().is_empty())
        .map(|address| tokio::spawn(async move { probe_one(address).await }))
        .collect();
    let mut out = Vec::with_capacity(tasks.len());
    for task in tasks {
        if let Ok(entry) = task.await {
            out.push(entry);
        }
    }
    out
}

async fn probe_one(address: String) -> PathProbeEntry {
    probe_addr(address, crate::ssh::types::DEFAULT_PORT).await
}

async fn probe_addr(address: String, port: u16) -> PathProbeEntry {
    let started = Instant::now();
    let connect = TcpStream::connect((address.as_str(), port));
    match tokio::time::timeout(PATH_PROBE_TIMEOUT, connect).await {
        Ok(Ok(_stream)) => PathProbeEntry {
            address,
            reachable: true,
            rtt_ms: Some(started.elapsed().as_millis().min(u64::MAX as u128) as u64),
        },
        _ => PathProbeEntry {
            address,
            reachable: false,
            rtt_ms: None,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn probe_reports_reachability_and_rtt() {
        // One live listener and one black hole (TEST-NET, effectively
        // unreachable). probe_addr is the raw unit; the command dials :22.
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        tokio::spawn(async move {
            // Keep accepting so the probe's connect() succeeds.
            while let Ok(_) = listener.accept().await {}
        });

        // A definitely-closed port: bind, note the port, drop the listener.
        let closed = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let closed_port = closed.local_addr().unwrap().port();
        drop(closed);

        let live = probe_addr("127.0.0.1".into(), port).await;
        assert!(live.reachable);
        assert!(live.rtt_ms.is_some());

        let dead = probe_addr("127.0.0.1".into(), closed_port).await;
        assert!(!dead.reachable);
        assert!(dead.rtt_ms.is_none());
    }

    #[tokio::test]
    async fn probe_one_measures_connect_rtt() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        tokio::spawn(async move {
            while let Ok(_) = listener.accept().await {}
        });
        let entry = probe_addr("127.0.0.1".into(), port).await;
        assert!(entry.reachable);
        assert!(entry.rtt_ms.unwrap() < 1000);
    }
}
