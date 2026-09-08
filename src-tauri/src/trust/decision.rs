//! Trust decision enum (r2 §6.13). The UI renders ONLY from this enum.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TrustDecision {
    /// Same host key + same product identity → proceed silently.
    Trusted,
    /// Never seen before → TOFU, capture after successful auth.
    FirstSeen,
    /// Host key changed but product identity unchanged → SSH keys were
    /// likely regenerated. Requires explicit user verification.
    HostKeyChangedNeedsVerification,
    /// Host key changed AND system identity changed, but the hardware serial
    /// matches → the Jetson was likely re-flashed. Re-enroll, but never
    /// silently.
    ReEnrollSameHardware,
    /// Everything changed → this address is likely a different machine.
    NewDeviceAtKnownAddress,
    /// Same host key but product identity changed/conflicted → clone or
    /// copied image. Needs user decision.
    IdentityConflict,
    /// No host key available at all → cannot establish trust. Block.
    Blocked,
}

impl TrustDecision {
    /// Whether an automatic reconnect may proceed with a saved password.
    /// Only `Trusted`/`FirstSeen` qualify (r2 §6.6/§6.10 iron rule).
    pub fn allows_auto_reconnect(self) -> bool {
        matches!(self, TrustDecision::Trusted | TrustDecision::FirstSeen)
    }

    /// Whether the saved credential may be used automatically.
    pub fn allows_saved_credential(self) -> bool {
        self.allows_auto_reconnect()
    }
}
