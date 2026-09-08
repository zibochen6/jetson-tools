//! Stable machine-readable error codes (Reliability Harness Phase 0).
//!
//! Rules (from ChatGPT execution package r2 §2.6 + §3.10):
//! - Every user-visible failure MUST have a stable `ErrorCode`. The frontend
//!   must never parse human-readable error strings for logic — only codes.
//! - An `ErrorCode` is a bare enum variant: it physically cannot carry
//!   credentials, tokens, argv or any string payload. Details travel through
//!   a separate sanitized channel elsewhere; never through this type.
//! - Codes serialize to snake_case lowercase and are stable once shipped.
//!
//! Dev-facing notes for new codes: append only, never rename/renumber.

use serde::{Deserialize, Serialize};
use std::fmt;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ErrorCode {
    /// SSH TCP connect / banner read timed out (device off, wrong IP, firewalled).
    SshTimeout,
    /// SSH authentication rejected (wrong password or username).
    SshAuthFailed,
    /// sshd presents a host key that differs from the TOFU record — the
    /// device-trust gate blocks auto-reconnect (see updater-trust task).
    SshHostKeyChanged,
    /// Device is unreachable at the network level.
    DeviceUnreachable,
    /// The remote is reachable over SSH but is not a Jetson.
    NotAJetson,
    /// sudo credentials rejected or sudo unavailable.
    SudoFailed,
    /// A provisioning step failed (check/apply/verify did not pass).
    ProvisionStepFailed,
    /// The local ssh tunnel could not be started (ports, binary, askpass).
    TunnelStartFailed,
    /// The tunnel process is up but the far end (Jetson :22) is unreachable.
    TunnelTargetUnreachable,
    /// russh over the loopback tunnel failed (tunnel up, hop broken).
    LoopbackSshFailed,
    /// RDP handshake / authentication / transport failed.
    RdpConnectFailed,
    /// The embedded FreeRDP engine crashed or exited unexpectedly.
    RdpEngineCrashed,
    /// Anything not yet classified. Must NOT be used when a specific code
    /// exists; treat hits as a bug to fix upstream.
    Unknown,
}

impl ErrorCode {
    /// Whether a retry with the same inputs has a chance of succeeding.
    /// Conservative by default: only clearly-transient classes are true.
    pub fn retryable(self) -> bool {
        matches!(
            self,
            ErrorCode::SshTimeout
                | ErrorCode::DeviceUnreachable
                | ErrorCode::TunnelTargetUnreachable
                | ErrorCode::LoopbackSshFailed
                | ErrorCode::RdpConnectFailed
        )
    }

    /// Stable world-facing code (identical to the serde snake_case name).
    pub fn as_str(self) -> &'static str {
        match self {
            ErrorCode::SshTimeout => "ssh_timeout",
            ErrorCode::SshAuthFailed => "ssh_auth_failed",
            ErrorCode::SshHostKeyChanged => "ssh_host_key_changed",
            ErrorCode::DeviceUnreachable => "device_unreachable",
            ErrorCode::NotAJetson => "not_a_jetson",
            ErrorCode::SudoFailed => "sudo_failed",
            ErrorCode::ProvisionStepFailed => "provision_step_failed",
            ErrorCode::TunnelStartFailed => "tunnel_start_failed",
            ErrorCode::TunnelTargetUnreachable => "tunnel_target_unreachable",
            ErrorCode::LoopbackSshFailed => "loopback_ssh_failed",
            ErrorCode::RdpConnectFailed => "rdp_connect_failed",
            ErrorCode::RdpEngineCrashed => "rdp_engine_crashed",
            ErrorCode::Unknown => "unknown",
        }
    }
}

impl fmt::Display for ErrorCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn serde_roundtrip_is_snake_case() {
        let cases: &[(ErrorCode, &str)] = &[
            (ErrorCode::SshTimeout, "ssh_timeout"),
            (ErrorCode::SshAuthFailed, "ssh_auth_failed"),
            (ErrorCode::SshHostKeyChanged, "ssh_host_key_changed"),
            (ErrorCode::DeviceUnreachable, "device_unreachable"),
            (ErrorCode::NotAJetson, "not_a_jetson"),
            (ErrorCode::SudoFailed, "sudo_failed"),
            (ErrorCode::ProvisionStepFailed, "provision_step_failed"),
            (ErrorCode::TunnelStartFailed, "tunnel_start_failed"),
            (
                ErrorCode::TunnelTargetUnreachable,
                "tunnel_target_unreachable",
            ),
            (ErrorCode::LoopbackSshFailed, "loopback_ssh_failed"),
            (ErrorCode::RdpConnectFailed, "rdp_connect_failed"),
            (ErrorCode::RdpEngineCrashed, "rdp_engine_crashed"),
            (ErrorCode::Unknown, "unknown"),
        ];
        for (code, name) in cases {
            let json = serde_json::to_string(code).expect("serialize");
            assert_eq!(json, format!("\"{name}\""));
            let back: ErrorCode = serde_json::from_str(&json).expect("deserialize");
            assert_eq!(back, *code);
            assert_eq!(code.as_str(), *name);
            assert_eq!(code.to_string(), *name);
        }
    }

    #[test]
    fn codes_are_bare_enums_no_payload() {
        // Compile-time-ish guard: an ErrorCode has no string field, so a
        // credential can never ride inside one. Clone it and serialize — the
        // only bytes emitted are the enum name.
        let code = ErrorCode::SshAuthFailed;
        let json = serde_json::to_string(&code).unwrap();
        assert_eq!(json, "\"ssh_auth_failed\"");
        // Same code from a different site is indistinguishable: nothing is
        // smuggled in a variant.
        assert_eq!(code, code);
    }

    #[test]
    fn retryable_set_is_deliberate() {
        assert!(ErrorCode::SshTimeout.retryable());
        assert!(ErrorCode::TunnelTargetUnreachable.retryable());
        assert!(
            !ErrorCode::SshAuthFailed.retryable(),
            "wrong password will not fix itself"
        );
        assert!(!ErrorCode::NotAJetson.retryable());
        assert!(!ErrorCode::SudoFailed.retryable());
        assert!(
            !ErrorCode::SshHostKeyChanged.retryable(),
            "trust gate requires user decision"
        );
        assert!(
            !ErrorCode::Unknown.retryable(),
            "unclassified failures must not auto-retry"
        );
    }
}
