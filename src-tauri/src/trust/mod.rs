//! Device trust (Reliability task: updater-trust, r2 §6).
//!
//! Two identities are deliberately separated:
//! - **SSH host identity** (host key fingerprint) — the security gate.
//! - **Jetson product identity** (device-tree serial / machine-id) — product
//!   logic for grouping and remembering devices.
//!
//! `evaluate_trust` is the pure decision core: the frontend/backend must never
//! re-implement "host changed AND machine changed → …" business logic.
//! `TrustStoreFile` (`store.rs`) is the pre-existing persistent TOFU store
//! (hosts.json, non-secret metadata only).

pub mod decision;
pub mod evaluator;
pub mod model;
mod store;

pub use decision::TrustDecision;
pub use evaluator::evaluate_trust;
pub use model::{DeviceIdentity, HostKeyRecord, TrustRecord};
pub use store::TrustStoreFile;
