//! Session layer contract models (r2 §4). Pure data, no pointers, no
//! FreeRDP/AppKit types — this module is the interface freeze boundary.

use std::sync::Arc;

/// Desktop geometry in logical pixels (r2 §4.3).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DesktopSize {
    pub width: u32,
    pub height: u32,
}

impl DesktopSize {
    pub const fn new(width: u32, height: u32) -> Self {
        Self { width, height }
    }
}

/// A clipboard text payload (text-only, per KI-019/MVP scope).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClipboardPayload {
    pub text: String,
}

/// Input event forwarded to the engine. Pure data only.
#[derive(Debug, Clone, PartialEq)]
pub enum RdpInputEvent {
    MouseMove {
        x: i32,
        y: i32,
    },
    MouseButton {
        button: u8,
        pressed: bool,
        x: i32,
        y: i32,
    },
    Key {
        scancode: u32,
        pressed: bool,
        extended: bool,
    },
    UnicodeText(String),
    ResetModifiers,
}

/// A session's externally visible state (r2 §4.12 snapshot projection —
/// the UI renders only this).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SessionViewState {
    Launching,
    Connecting,
    Connected,
    Error,
    Disconnected,
}

#[derive(Debug, Clone)]
pub struct SessionSnapshot {
    pub session_id: String,
    pub device_id: String,
    pub state: SessionViewState,
    pub active: bool,
    pub attempt_id: u64,
    pub error: Option<String>,
}

/// Event sink installed on the engine at start (r2 §4.5).
pub trait RdpEventSink: Send + Sync {
    fn on_event(
        &self,
        session_id: &str,
        attempt_id: u64,
        event: crate::session::events::RdpEngineEvent,
    );
}

/// Type alias kept short for signatures.
pub type SharedEventSink = Arc<dyn RdpEventSink>;
