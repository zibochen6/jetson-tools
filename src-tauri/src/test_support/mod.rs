//! Shared test primitives (Reliability Harness Phase 1).
//!
//! Per ChatGPT execution package r2 §8 Phase 1: one set of fakes and fault
//! tools used by every reliability test, instead of each module inventing its
//! own. Production code must never depend on this module for behavior — the
//! fault injector is a no-op in release builds by construction.

pub mod fake_clock;
pub mod fault;
pub mod recorder;
