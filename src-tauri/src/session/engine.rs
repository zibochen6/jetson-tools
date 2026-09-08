//! `RdpEngine` trait — the frozen adapter boundary (r2 §4.3–§4.5).
//!
//! Contract:
//! - No FreeRDP types (`freerdp*`, `rdpContext*`, CLIPRDR/GDI handles), no
//!   AppKit types (`NSView*`, `CALayer*`), no `void*` anywhere in this layer.
//! - The engine receives an opaque surface handle it must not interpret.
//! - Errors surface as stable `ErrorCode` variants.

use super::events::RdpEngineEvent;
use super::model::{ClipboardPayload, DesktopSize, RdpInputEvent, SharedEventSink};
/// Opaque native surface handle. Constructed by the platform layer (see
/// design §2); engines treat it as a token.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct NativeSurfaceHandle(pub u64);

impl NativeSurfaceHandle {
    pub const fn new(id: u64) -> Self {
        Self(id)
    }
}

/// Opaque engine-owned session handle (r2 §4.3).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct RdpEngineHandle(pub u64);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RdpEngineHealth {
    Running,
    RunningNoFrame,
    Stopped,
}

pub trait RdpEngine: Send {
    fn start(
        &mut self,
        session_id: &str,
        attempt_id: u64,
        config: &rds::RdpConnectionConfig,
        surface: NativeSurfaceHandle,
        sink: SharedEventSink,
    ) -> Result<RdpEngineHandle, crate::error_code::ErrorCode>;

    fn stop(&mut self, handle: &RdpEngineHandle) -> Result<(), crate::error_code::ErrorCode>;

    fn resize(
        &mut self,
        handle: &RdpEngineHandle,
        size: DesktopSize,
    ) -> Result<(), crate::error_code::ErrorCode>;

    fn set_focus(
        &mut self,
        handle: &RdpEngineHandle,
        focused: bool,
    ) -> Result<(), crate::error_code::ErrorCode>;

    fn send_input(
        &mut self,
        handle: &RdpEngineHandle,
        event: RdpInputEvent,
    ) -> Result<(), crate::error_code::ErrorCode>;

    fn set_clipboard(
        &mut self,
        handle: &RdpEngineHandle,
        payload: ClipboardPayload,
    ) -> Result<(), crate::error_code::ErrorCode>;

    fn health(&mut self, handle: &RdpEngineHandle) -> RdpEngineHealth;
}

/// Re-export for consumers (keeps signatures short).
mod rds {
    pub use crate::rdp::types::RdpConnectionConfig;
}
pub use rds::RdpConnectionConfig;

impl RdpEngineEvent {
    pub fn is_terminal(&self) -> bool {
        matches!(
            self,
            RdpEngineEvent::Disconnected { .. } | RdpEngineEvent::Error { .. }
        )
    }
}
