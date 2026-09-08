//! Frozen session contract layer (r2 §4 — Embedded Session Architecture).
//!
//! Boundary rules:
//! - `engine.rs` is the ONLY place `RdpEngine` is defined; the trait leaks no
//!   FreeRDP/AppKit types (opaque handles only).
//! - `manager.rs` owns lifecycle policy: focus exclusivity, clipboard
//!   ownership, deterministic cleanup order, attempt generations.
//! - `fake.rs` is the deterministic test engine; production adapters live in
//!   `crate::rdp` and implement the same trait.

pub mod engine;
pub mod events;
pub mod fake;
pub mod manager;
pub mod model;
