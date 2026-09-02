use serde::Serialize;
use thiserror::Error;

/// Internal RDP-layer error. Mapped to a typed, sanitized IPC error at the
/// command boundary — FreeRDP's raw stderr is never surfaced to the frontend.
#[derive(Debug, Error)]
pub enum RdpError {
    #[error("FreeRDP client not found")]
    ClientNotFound,
    #[error("FreeRDP version is unsupported")]
    VersionUnsupported,
    #[error("failed to launch FreeRDP: {0}")]
    LaunchFailed(#[from] std::io::Error),
    #[error("no stored password is available for this device")]
    PasswordMissing,
    #[error("rdp error")]
    Unknown,
}

/// Typed IPC error code (PRD §25). The reserved variants are part of the
/// contract so the frontend can map them without a schema change; only the
/// first three are produced in Phase 4A (deep auth/cert/connection detection is
/// deferred — the process status we can honestly observe is spawn + exit).
#[derive(Debug, Serialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum RdpErrorCode {
    RdpClientNotFound,
    RdpClientVersionUnsupported,
    RdpLaunchFailed,
    #[allow(dead_code)]
    RdpAuthenticationFailed,
    #[allow(dead_code)]
    RdpCertificateChanged,
    #[allow(dead_code)]
    RdpConnectionFailed,
    #[allow(dead_code)]
    RdpProcessCrashed,
    #[allow(dead_code)]
    RdpAlreadyRunning,
    RdpPasswordMissing,
    RdpUnknown,
}

/// Typed, sanitized IPC error — never a raw OS/`io::Error` string.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RdpIpcError {
    pub code: RdpErrorCode,
    pub message: String,
}

pub fn map_rdp_error(e: RdpError) -> RdpIpcError {
    let (code, message) = match e {
        RdpError::ClientNotFound => (
            RdpErrorCode::RdpClientNotFound,
            "FreeRDP is required for this development build.",
        ),
        RdpError::VersionUnsupported => (
            RdpErrorCode::RdpClientVersionUnsupported,
            "The installed FreeRDP version is not supported.",
        ),
        RdpError::LaunchFailed(_) => (
            RdpErrorCode::RdpLaunchFailed,
            "Couldn't open the Jetson desktop.",
        ),
        RdpError::PasswordMissing => (
            RdpErrorCode::RdpPasswordMissing,
            "No stored password is available for this device",
        ),
        RdpError::Unknown => (
            RdpErrorCode::RdpUnknown,
            "Couldn't open the Jetson desktop.",
        ),
    };
    RdpIpcError {
        code,
        message: message.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_client_not_found() {
        let e = map_rdp_error(RdpError::ClientNotFound);
        assert!(matches!(e.code, RdpErrorCode::RdpClientNotFound));
        // message never leaks secrets / internals
        assert!(!e.message.contains("error"));
    }

    #[test]
    fn launch_failed_drops_inner_io_string() {
        let io = std::io::Error::other("secret-path/tok");
        let e = map_rdp_error(RdpError::LaunchFailed(io));
        assert!(matches!(e.code, RdpErrorCode::RdpLaunchFailed));
        assert!(!e.message.contains("secret-path"));
    }
}
