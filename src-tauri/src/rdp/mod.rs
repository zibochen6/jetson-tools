//! Desktop Plane (RDP) — the FreeRDP sidecar that renders the Jetson desktop.
//!
//! Phase 4A launches the system `sdl-freerdp` as an independent native window
//! (no FFI, no framebuffer embedding). The `RdpClient` abstraction is the
//! future seam for an embedded/WebRTC transport (see ARCHITECTURE §5).
//!
//! Credential rule: the password travels React → Rust memory → `sdl-freerdp`
//! stdin only. It is never in `argv`, never logged, never persisted.

pub mod args;
pub mod client;
pub mod error;
pub mod ffi;
pub mod freerdp;
pub mod manager;
pub mod native_view;
pub mod process;
pub mod session;
pub mod types;
