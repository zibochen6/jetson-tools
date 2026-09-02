//! Raw TCP reachability probe.
//!
//! Returns the OS errno verbatim so we can tell a macOS Local Network (TCC)
//! block (ENETUNREACH / EHOSTUNREACH, errno 51/65) apart from a real transport
//! failure (ECONNREFUSED 61 / ETIMEDOUT 60). Deliberately NOT mapped to the
//! product's generic `ProbeErrorCode` — this is a diagnostic surface.

use serde::Serialize;
use std::net::{TcpStream, ToSocketAddrs};
use std::time::Duration;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TcpProbe {
    pub host: String,
    pub port: u16,
    pub connected: bool,
    pub error_kind: Option<String>,
    pub os_errno: Option<i32>,
    pub detail: String,
}

pub fn tcp_probe(host: &str, port: u16, timeout: Duration) -> TcpProbe {
    // Numeric IPs resolve without DNS; a hostname that fails here is a distinct
    // signal from a blocked `connect()`.
    let addr = match (host, port).to_socket_addrs() {
        Ok(mut addrs) => match addrs.next() {
            Some(addr) => addr,
            None => {
                return TcpProbe {
                    host: host.to_string(),
                    port,
                    connected: false,
                    error_kind: Some("ResolutionEmpty".to_string()),
                    os_errno: None,
                    detail: "host resolved to no socket address".to_string(),
                }
            }
        },
        Err(e) => {
            return TcpProbe {
                host: host.to_string(),
                port,
                connected: false,
                error_kind: Some(format!("{:?}", e.kind())),
                os_errno: e.raw_os_error(),
                detail: e.to_string(),
            }
        }
    };

    match TcpStream::connect_timeout(&addr, timeout) {
        Ok(_stream) => TcpProbe {
            host: host.to_string(),
            port,
            connected: true,
            error_kind: None,
            os_errno: None,
            detail: format!("connected to {addr}"),
        },
        Err(e) => TcpProbe {
            host: host.to_string(),
            port,
            connected: false,
            error_kind: Some(format!("{:?}", e.kind())),
            os_errno: e.raw_os_error(),
            detail: e.to_string(),
        },
    }
}

/// Human categorization for the final report: which failure class this errno is.
pub fn classify(os_errno: Option<i32>) -> &'static str {
    match os_errno {
        Some(51) => "ENETUNREACH",
        Some(65) => "EHOSTUNREACH (No route to host)",
        Some(61) => "ECONNREFUSED",
        Some(60) => "ETIMEDOUT",
        Some(1) => "EPERM (blocked?)",
        Some(_) => "other errno",
        None => "no errno (resolution/hang?)",
    }
}
