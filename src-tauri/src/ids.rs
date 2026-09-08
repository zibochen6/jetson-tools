//! Stable ID newtypes (Reliability Harness Phase 0).
//!
//! Every long-running asynchronous flow must carry its own generation so a
//! stale callback can never contaminate a newer attempt (KI-024/KI-036 class
//! of bugs). The four IDs below are the project-wide vocabulary:
//!
//! - `SessionId` — one live desktop session (RDP + tunnel + credentials).
//! - `AttemptId` — one connection attempt, monotonically increasing
//!   in-process; callbacks capture it and discard themselves when stale.
//! - `RunId` — one provisioning run (bootstrap JSONL `run_id`).
//! - `TransactionId` — one updater transaction.

use std::fmt;
use std::sync::atomic::{AtomicU64, Ordering};

use serde::{Deserialize, Serialize};

macro_rules! string_id {
    ($(#[$doc:meta])* $name:ident) => {
        $(#[$doc])*
        #[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
        pub struct $name(pub String);

        impl $name {
            pub fn new(value: impl Into<String>) -> Self {
                Self(value.into())
            }

            pub fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                self.0.fmt(f)
            }
        }
    };
}

string_id!(
    /// One live desktop session, keyed by `username@deviceId` on the frontend.
    SessionId
);
string_id!(
    /// One provisioning run. Prefer a short random token so logs stay
    /// grep-able (e.g. `r-7f3a9c`).
    RunId
);
string_id!(
    /// One updater transaction (journal directory name).
    TransactionId
);

/// One connection attempt. Monotonically increasing within a process run;
/// the existing-sequence check is the stale-callback guard.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct AttemptId(pub u64);

impl AttemptId {
    /// Allocate the next attempt id (1-based).
    pub fn next() -> Self {
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        Self(COUNTER.fetch_add(1, Ordering::Relaxed) + 1)
    }

    pub fn value(self) -> u64 {
        self.0
    }
}

impl fmt::Display for AttemptId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "a{}", self.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn attempt_id_is_monotonic() {
        let a = AttemptId::next();
        let b = AttemptId::next();
        let c = AttemptId::next();
        assert!(a < b && b < c, "attempt ids must strictly increase");
    }

    #[test]
    fn attempt_id_serde_roundtrip() {
        let id = AttemptId(42);
        let json = serde_json::to_string(&id).expect("serialize");
        assert_eq!(json, "42");
        let back: AttemptId = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(back, id);
    }

    #[test]
    fn string_ids_serde_roundtrip_and_display() {
        let sid = SessionId::new("seeed@1421123007848");
        let json = serde_json::to_string(&sid).expect("serialize");
        assert_eq!(json, "\"seeed@1421123007848\"");
        let back: SessionId = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(back, sid);
        assert_eq!(sid.to_string(), "seeed@1421123007848");
        assert_eq!(sid.as_str(), "seeed@1421123007848");
    }

    #[test]
    fn run_id_and_transaction_id_display() {
        assert_eq!(RunId::new("r-7f3a9c").to_string(), "r-7f3a9c");
        assert_eq!(
            TransactionId::new("01JUPDATEABC").to_string(),
            "01JUPDATEABC"
        );
    }

    #[test]
    fn ids_never_carry_credentials_by_construction() {
        // The ID types hold a single plain String. There is no field that can
        // hold a password — this test exists to keep the module surface tiny.
        let run_id = RunId::new("valid-characters-only-token");
        assert!(!run_id.as_str().contains('\n'));
    }
}
