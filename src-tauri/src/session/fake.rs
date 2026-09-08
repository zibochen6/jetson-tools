//! FakeRdpAdapter — deterministic in-process engine for lifecycle tests
//! (r2 §4.14). Scripts a session's behavior and records every call so the
//! SessionManager cleanup/focus/attempt contracts are asserted exactly.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use super::engine::{NativeSurfaceHandle, RdpEngine, RdpEngineHandle, RdpEngineHealth};
use super::events::RdpEngineEvent;
use super::model::{ClipboardPayload, DesktopSize, RdpInputEvent, SharedEventSink};
use crate::error_code::ErrorCode;

pub type FakeScenario = Vec<FakeStep>;

#[derive(Debug, Clone)]
pub enum FakeStep {
    /// Emit Connected with this size when start() is called.
    ConnectSuccess { size: DesktopSize },
    /// start() fails with this code.
    ConnectError { code: ErrorCode },
    /// Emit a Disconnected/Transport after N events have been consumed.
    DisconnectAfter { events: u64 },
    /// Emit an EngineExit error event.
    EngineCrash,
}

#[derive(Debug, Clone)]
pub enum FakeCall {
    Start {
        session_id: String,
        attempt_id: u64,
        surface: NativeSurfaceHandle,
    },
    Stop {
        handle: RdpEngineHandle,
    },
    Resize {
        handle: RdpEngineHandle,
        size: DesktopSize,
    },
    SetFocus {
        handle: RdpEngineHandle,
        focused: bool,
    },
    SendInput {
        handle: RdpEngineHandle,
        event: RdpInputEvent,
    },
    SetClipboard {
        handle: RdpEngineHandle,
        payload: ClipboardPayload,
    },
}

pub struct FakeRdpAdapter {
    pub calls: Vec<FakeCall>,
    pub scenario: FakeScenario,
    attempts: AtomicU64,
}

impl Default for FakeRdpAdapter {
    fn default() -> Self {
        Self {
            calls: Vec::new(),
            scenario: vec![FakeStep::ConnectSuccess {
                size: DesktopSize::new(1280, 800),
            }],
            attempts: AtomicU64::new(0),
        }
    }
}

impl FakeRdpAdapter {
    pub fn with_scenario(scenario: FakeScenario) -> Self {
        Self {
            scenario,
            ..Self::default()
        }
    }
    pub fn calls_snapshot(&self) -> Vec<FakeCall> {
        self.calls.clone()
    }
}

pub struct FakeEvents;

impl crate::session::model::RdpEventSink for FakeEvents {
    fn on_event(&self, _s: &str, _a: u64, _e: RdpEngineEvent) {}
}

impl RdpEngine for FakeRdpAdapter {
    fn start(
        &mut self,
        session_id: &str,
        attempt_id: u64,
        _config: &crate::rdp::types::RdpConnectionConfig,
        surface: NativeSurfaceHandle,
        sink: SharedEventSink,
    ) -> Result<RdpEngineHandle, ErrorCode> {
        self.calls.push(FakeCall::Start {
            session_id: session_id.into(),
            attempt_id,
            surface,
        });
        let handle = RdpEngineHandle(self.attempts.fetch_add(1, Ordering::Relaxed));
        for step in &self.scenario {
            match step {
                FakeStep::ConnectSuccess { size } => {
                    sink.on_event(
                        session_id,
                        attempt_id,
                        RdpEngineEvent::Connected {
                            desktop_size: *size,
                        },
                    );
                }
                FakeStep::ConnectError { code } => return Err(*code),
                FakeStep::DisconnectAfter { .. } => {} // driven by the manager/test
                FakeStep::EngineCrash => sink.on_event(
                    session_id,
                    attempt_id,
                    RdpEngineEvent::Error {
                        code: ErrorCode::RdpEngineCrashed,
                        retryable: false,
                    },
                ),
            }
        }
        Ok(handle)
    }

    fn stop(&mut self, handle: &RdpEngineHandle) -> Result<(), ErrorCode> {
        self.calls.push(FakeCall::Stop { handle: *handle });
        Ok(())
    }

    fn resize(&mut self, handle: &RdpEngineHandle, size: DesktopSize) -> Result<(), ErrorCode> {
        self.calls.push(FakeCall::Resize {
            handle: *handle,
            size,
        });
        Ok(())
    }

    fn set_focus(&mut self, handle: &RdpEngineHandle, focused: bool) -> Result<(), ErrorCode> {
        self.calls.push(FakeCall::SetFocus {
            handle: *handle,
            focused,
        });
        Ok(())
    }

    fn send_input(
        &mut self,
        handle: &RdpEngineHandle,
        event: RdpInputEvent,
    ) -> Result<(), ErrorCode> {
        self.calls.push(FakeCall::SendInput {
            handle: *handle,
            event,
        });
        Ok(())
    }

    fn set_clipboard(
        &mut self,
        handle: &RdpEngineHandle,
        payload: ClipboardPayload,
    ) -> Result<(), ErrorCode> {
        self.calls.push(FakeCall::SetClipboard {
            handle: *handle,
            payload,
        });
        Ok(())
    }

    fn health(&mut self, _handle: &RdpEngineHandle) -> RdpEngineHealth {
        RdpEngineHealth::Running
    }
}

/// Arc wrapper for sharing one adapter across the manager + assertion site.
pub type SharedFakeRdp = Arc<Mutex<FakeRdpAdapter>>;
