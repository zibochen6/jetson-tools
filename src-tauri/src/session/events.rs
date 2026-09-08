//! Engine event surface (r2 §4.5). Nothing below exposes FreeRDP details.

use super::model::{ClipboardPayload, DesktopSize};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RdpDisconnectReason {
    /// Clean local/user-initiated disconnect.
    Local,
    /// The remote closed the connection.
    RemoteClosed,
    /// Transport/network failure.
    Transport,
    /// The engine itself died.
    EngineExit,
}

#[derive(Debug, Clone, PartialEq)]
pub enum RdpEngineEvent {
    Connecting,
    Connected {
        desktop_size: DesktopSize,
    },
    Disconnected {
        reason: RdpDisconnectReason,
    },
    Error {
        /// Stable error code (no credentials by construction, see error_code.rs).
        code: crate::error_code::ErrorCode,
        retryable: bool,
    },
    ClipboardOffer,
    ClipboardData {
        payload: ClipboardPayload,
    },
}
