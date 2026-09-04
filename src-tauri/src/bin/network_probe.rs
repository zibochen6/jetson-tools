// Headless raw-TCP network probe — same probe as the in-app `network_probe`
// command, runnable without the GUI for fast iteration.
// Usage: cargo run --bin network_probe -- <host> <port> [timeout_ms]
// NOTE: this is an UNSIGNED cargo binary (no bundle identity), so macOS Local
// Network privacy will silently block it the same way it blocks `cargo tauri dev`.

use std::env;
use std::time::Duration;

use jetson_remote_lib::net::probe::{classify, tcp_probe};

fn main() {
    let args: Vec<String> = env::args().collect();
    let host = args.get(1).map(String::as_str).unwrap_or("192.168.100.164");
    let port: u16 = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(22);
    let timeout_ms: u64 = args.get(3).and_then(|s| s.parse().ok()).unwrap_or(5000);

    let p = tcp_probe(host, port, Duration::from_millis(timeout_ms));
    let classification = classify(p.os_errno);
    println!(
        "{} {host}:{port} connected={} error_kind={:?} os_errno={:?} class={classification} detail={}",
        if p.connected { "PASS" } else { "FAIL" },
        p.connected,
        p.error_kind,
        p.os_errno,
        p.detail,
    );
    if !p.connected {
        std::process::exit(1);
    }
}
