//! `FreeRdpEngineAdapter` — production adapter binding the existing,
//! real-device-verified `RdpSessionManager` (native FreeRDP bridge) to the
//! frozen `RdpEngine` trait.
//!
//! Status: compiled + linked (trait contract), NOT yet part of the live
//! command path. Switching the live path over is the real-hardware gate
//! (see .trellis/tasks/09-08-embedded-session-arch prd "接线清单").

use std::sync::{Arc, Mutex};

use crate::error_code::ErrorCode;
use crate::rdp::session::RdpSessionManager;
use crate::rdp::types::RdpConnectionConfig;
use crate::session::engine::{NativeSurfaceHandle, RdpEngine, RdpEngineHandle, RdpEngineHealth};
use crate::session::model::{ClipboardPayload, DesktopSize, RdpInputEvent, SharedEventSink};

pub struct FreeRdpEngineAdapter {
    /// The existing manager (windowed). This adapter is a thin contract
    /// bridge; window/surface mgmt stays with the caller today.
    inner: Arc<Mutex<RdpSessionManager>>,
    /// Maps RdpEngineHandle → session key currently known to the manager.
    keys: Vec<String>,
}

impl FreeRdpEngineAdapter {
    pub fn new(inner: Arc<Mutex<RdpSessionManager>>) -> Self {
        Self {
            inner,
            keys: Vec::new(),
        }
    }

    fn key(&self, handle: &RdpEngineHandle) -> &str {
        self.keys
            .get(handle.0 as usize)
            .map(String::as_str)
            .unwrap_or("")
    }
}

impl RdpEngine for FreeRdpEngineAdapter {
    fn start(
        &mut self,
        session_id: &str,
        _attempt_id: u64,
        _config: &RdpConnectionConfig,
        _surface: NativeSurfaceHandle,
        _sink: SharedEventSink,
    ) -> Result<RdpEngineHandle, ErrorCode> {
        // Production start includes window mounting + insets + credentials —
        // those remain the command layer's job until the real-hardware
        // wiring step. The adapter stores the key so later trait calls can
        // address the manager.
        let mut key_index = self.keys.iter().position(|k| k == session_id);
        if key_index.is_none() {
            self.keys.push(session_id.to_string());
            key_index = Some(self.keys.len() - 1);
        }
        let idx = key_index.expect("just set") as u64;
        // No-op spawn: live launch stays in commands/rdp.rs `launch_session`.
        let _guard = self.inner.lock();
        Ok(RdpEngineHandle(idx))
    }

    fn stop(&mut self, handle: &RdpEngineHandle) -> Result<(), ErrorCode> {
        let key = self.key(handle);
        if key.is_empty() {
            return Err(ErrorCode::Unknown);
        }
        // Live close is async over Tokio; the trait surface is synchronous.
        // NOTE real-hardware wiring step will bridge through a blocking
        // runtime handle; for the frozen contract this is an explicit no-op
        // so the adapter stays honest about its pending state.
        let _ = key;
        Ok(())
    }

    fn resize(&mut self, _handle: &RdpEngineHandle, _size: DesktopSize) -> Result<(), ErrorCode> {
        Ok(()) // session geometry handled at launch today
    }

    fn set_focus(&mut self, _handle: &RdpEngineHandle, _focused: bool) -> Result<(), ErrorCode> {
        Ok(())
    }

    fn send_input(
        &mut self,
        _handle: &RdpEngineHandle,
        _event: RdpInputEvent,
    ) -> Result<(), ErrorCode> {
        Ok(())
    }

    fn set_clipboard(
        &mut self,
        _handle: &RdpEngineHandle,
        _payload: ClipboardPayload,
    ) -> Result<(), ErrorCode> {
        Ok(())
    }

    fn health(&mut self, _handle: &RdpEngineHandle) -> RdpEngineHealth {
        RdpEngineHealth::Running
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn adapter_compiles_and_returns_handle_for_start() {
        let manager = Arc::new(Mutex::new(RdpSessionManager::new()));
        let mut adapter = FreeRdpEngineAdapter::new(manager);
        let cfg = RdpConnectionConfig {
            certificate_name: "t".into(),
            host: "h".into(),
            port: 3389,
            username: "u".into(),
            password: "p".into(),
            dynamic_resolution: true,
            clipboard: true,
        };
        struct S;
        impl crate::session::model::RdpEventSink for S {
            fn on_event(&self, _s: &str, _a: u64, _e: crate::session::events::RdpEngineEvent) {}
        }
        let sink: SharedEventSink = Arc::new(S);
        let handle = adapter
            .start("s1", 1, &cfg, NativeSurfaceHandle::new(9), sink)
            .expect("adapter start");
        assert_eq!(handle, RdpEngineHandle(0));
        assert_eq!(adapter.health(&handle), RdpEngineHealth::Running);
    }
}
