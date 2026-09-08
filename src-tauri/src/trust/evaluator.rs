//! Pure trust decision function (r2 §6.6/§6.9 decision matrix).
//!
//! NO I/O, NO secrets. Inputs: what we remembered before (`previous`), what
//! the SSH handshake just saw (`observed_host_key`), and what device
//! detection read after login (`observed_identity`).

use super::decision::TrustDecision;
use super::model::{DeviceIdentity, HostKeyRecord, TrustRecord};

pub fn evaluate_trust(
    previous: Option<&TrustRecord>,
    logical_device_id: &str,
    observed_host_key: Option<&HostKeyRecord>,
    observed_identity: &DeviceIdentity,
) -> TrustDecision {
    // No host key at all → nothing to trust.
    let Some(host_key) = observed_host_key else {
        return TrustDecision::Blocked;
    };

    let Some(prev) = previous else {
        return TrustDecision::FirstSeen;
    };

    let host_key_ok = prev.matches_host_key(&host_key.fingerprint);

    let machine_same = same_opt(
        prev.machine_id.as_deref(),
        observed_identity.machine_id.as_deref(),
    );
    let serial_same = same_opt(prev.serial.as_deref(), observed_identity.serial.as_deref());

    // NOTE: logical_device_id is validated against the record's identity by
    // the caller (serial preferred). Here it only disambiguates records.
    let _ = logical_device_id;

    match (host_key_ok, machine_same, serial_same) {
        (true, true, true) => TrustDecision::Trusted,
        // A: host key regenerated, same machine → needs verification.
        (false, true, true) => TrustDecision::HostKeyChangedNeedsVerification,
        // B: re-flash (host key + machine-id changed, serial stable).
        (false, false, true) => TrustDecision::ReEnrollSameHardware,
        // Serial changed while other pieces differ → conflict, be conservative.
        (false, true, false) => TrustDecision::IdentityConflict,
        // C: everything changed → different machine at a known address.
        (false, false, false) => TrustDecision::NewDeviceAtKnownAddress,
        // D: same key, machine-id changed, serial same → re-enroll candidate.
        (true, false, true) => TrustDecision::ReEnrollSameHardware,
        // E: same key but both identities changed → clone/copied image.
        (true, true, false) => TrustDecision::IdentityConflict,
        (true, false, false) => TrustDecision::IdentityConflict,
    }
}

fn same_opt(a: Option<&str>, b: Option<&str>) -> bool {
    match (a, b) {
        (Some(x), Some(y)) => x == y,
        (None, None) => true,
        // One side no longer reports the value → treat as changed (do not
        // silently trust a missing identity field).
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record(machine: &str, serial: &str, keys: &[&str]) -> TrustRecord {
        TrustRecord {
            logical_device_id: serial.into(),
            machine_id: (!machine.is_empty()).then(|| machine.into()),
            serial: (!serial.is_empty()).then(|| serial.into()),
            trusted_host_keys: keys
                .iter()
                .map(|fp| HostKeyRecord {
                    algorithm: "ssh-ed25519".into(),
                    fingerprint: (*fp).into(),
                })
                .collect(),
            known_addresses: vec![],
            first_seen_at: None,
            last_seen_at: None,
        }
    }

    fn ident(machine: &str, serial: &str) -> DeviceIdentity {
        DeviceIdentity {
            machine_id: (!machine.is_empty()).then(|| machine.into()),
            serial: (!serial.is_empty()).then(|| serial.into()),
        }
    }

    fn key(fp: &str) -> HostKeyRecord {
        HostKeyRecord {
            algorithm: "ssh-ed25519".into(),
            fingerprint: fp.into(),
        }
    }

    #[test]
    fn first_seen_when_no_previous_record() {
        assert_eq!(
            evaluate_trust(None, "s1", Some(&key("key1")), &ident("m1", "s1")),
            TrustDecision::FirstSeen
        );
    }

    #[test]
    fn blocked_when_no_host_key() {
        assert_eq!(
            evaluate_trust(None, "s1", None, &ident("m1", "s1")),
            TrustDecision::Blocked
        );
    }

    #[test]
    fn same_key_same_identity_is_trusted() {
        let prev = record("m1", "s1", &["key1"]);
        assert_eq!(
            evaluate_trust(Some(&prev), "s1", Some(&key("key1")), &ident("m1", "s1")),
            TrustDecision::Trusted
        );
    }

    #[test]
    fn key_changed_same_identity_needs_verification() {
        let prev = record("m1", "s1", &["key1"]);
        assert_eq!(
            evaluate_trust(Some(&prev), "s1", Some(&key("key2")), &ident("m1", "s1")),
            TrustDecision::HostKeyChangedNeedsVerification
        );
    }

    #[test]
    fn key_and_machine_changed_same_serial_is_reenroll() {
        let prev = record("m1", "s1", &["key1"]);
        assert_eq!(
            evaluate_trust(Some(&prev), "s1", Some(&key("key2")), &ident("m2", "s1")),
            TrustDecision::ReEnrollSameHardware
        );
    }

    #[test]
    fn everything_changed_is_a_new_device_at_known_address() {
        let prev = record("m1", "s1", &["key1"]);
        assert_eq!(
            evaluate_trust(Some(&prev), "s1", Some(&key("key2")), &ident("m2", "s2")),
            TrustDecision::NewDeviceAtKnownAddress
        );
    }

    #[test]
    fn same_key_machine_changed_same_serial_is_reenroll() {
        let prev = record("m1", "s1", &["key1"]);
        assert_eq!(
            evaluate_trust(Some(&prev), "s1", Some(&key("key1")), &ident("m2", "s1")),
            TrustDecision::ReEnrollSameHardware
        );
    }

    #[test]
    fn same_key_all_identities_changed_is_conflict() {
        let prev = record("m1", "s1", &["key1"]);
        assert_eq!(
            evaluate_trust(Some(&prev), "s1", Some(&key("key1")), &ident("m2", "s2")),
            TrustDecision::IdentityConflict
        );
    }

    #[test]
    fn missing_serial_on_one_side_is_treated_as_changed() {
        let prev = record("m1", "s1", &["key1"]);
        // Serial disappeared (can't read device-tree) with same key+machine.
        assert_eq!(
            evaluate_trust(Some(&prev), "s1", Some(&key("key1")), &ident("m1", "")),
            TrustDecision::IdentityConflict
        );
    }

    #[test]
    fn auto_reconnect_and_saved_credential_only_on_trusted_first_seen() {
        assert!(TrustDecision::Trusted.allows_auto_reconnect());
        assert!(TrustDecision::FirstSeen.allows_auto_reconnect());
        assert!(TrustDecision::FirstSeen.allows_saved_credential());
        assert!(!TrustDecision::HostKeyChangedNeedsVerification.allows_auto_reconnect());
        assert!(!TrustDecision::ReEnrollSameHardware.allows_auto_reconnect());
        assert!(!TrustDecision::ReEnrollSameHardware.allows_saved_credential());
        assert!(!TrustDecision::NewDeviceAtKnownAddress.allows_auto_reconnect());
        assert!(!TrustDecision::IdentityConflict.allows_auto_reconnect());
        assert!(!TrustDecision::IdentityConflict.allows_saved_credential());
        assert!(!TrustDecision::Blocked.allows_auto_reconnect());
        assert!(!TrustDecision::Blocked.allows_saved_credential());
    }

    #[test]
    fn decision_matrix_serde_roundtrip() {
        for decision in [
            TrustDecision::Trusted,
            TrustDecision::FirstSeen,
            TrustDecision::HostKeyChangedNeedsVerification,
            TrustDecision::ReEnrollSameHardware,
            TrustDecision::NewDeviceAtKnownAddress,
            TrustDecision::IdentityConflict,
            TrustDecision::Blocked,
        ] {
            let json = serde_json::to_string(&decision).unwrap();
            let back: TrustDecision = serde_json::from_str(&json).unwrap();
            assert_eq!(back, decision);
        }
    }
}
