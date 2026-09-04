use thiserror::Error;

use super::types::JetsonDetectionResult;
use crate::ssh::error::SshError;
use crate::ssh::executor::RemoteExecutor;

/// `detect.sh` is embedded at compile time — the remote host needs nothing
/// pre-installed. See docs/SPIKE_RESULTS.md for the verified Phase 0 script.
pub const DETECT_SCRIPT: &str = include_str!("../../../scripts/remote/detect.sh");

#[derive(Debug, Error)]
pub enum DetectError {
    #[error(transparent)]
    Ssh(#[from] SshError),
    #[error("detection output was not valid JSON")]
    Parse,
}

#[derive(Debug)]
pub enum DetectOutcome {
    Device(JetsonDetectionResult),
    NotJetson,
}

/// Run the embedded detect.sh over an ephemeral SSH session via `sh -s` stdin
/// (no temporary files on the device) and parse a single JSON document from
/// stdout.
pub async fn detect<E: RemoteExecutor>(executor: &mut E) -> Result<DetectOutcome, DetectError> {
    let result = executor
        .exec_with_stdin("sh -s", DETECT_SCRIPT.as_bytes())
        .await?;

    if let Some(code) = result.exit_code {
        if code != 0 {
            return Err(SshError::CommandFailed(Some(code)).into());
        }
    }

    let stdout = String::from_utf8_lossy(&result.stdout);
    let parsed: JetsonDetectionResult =
        serde_json::from_str(stdout.trim()).map_err(|_| DetectError::Parse)?;

    if parsed.is_jetson {
        Ok(DetectOutcome::Device(parsed))
    } else {
        Ok(DetectOutcome::NotJetson)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ssh::executor::{RemoteCommandResult, RemoteExecutor};

    struct MockExec {
        stdout: Vec<u8>,
        exit_code: Option<u32>,
    }

    impl RemoteExecutor for MockExec {
        async fn exec(&mut self, _cmd: &str) -> Result<RemoteCommandResult, SshError> {
            Ok(self.result())
        }
        async fn exec_with_stdin(
            &mut self,
            _cmd: &str,
            _stdin: &[u8],
        ) -> Result<RemoteCommandResult, SshError> {
            Ok(self.result())
        }
    }

    impl MockExec {
        fn result(&self) -> RemoteCommandResult {
            RemoteCommandResult {
                stdout: self.stdout.clone(),
                stderr: Vec::new(),
                exit_code: self.exit_code,
            }
        }
    }

    // Real Phase-0 detect.sh output (see docs/SPIKE_RESULTS.md), trimmed to the
    // fields the detector consumes. serde ignores the rest.
    const FIXTURE: &str = r#"{
  "is_jetson": true,
  "architecture": "aarch64",
  "hostname": "seeed-desktop",
  "ubuntu_id": "ubuntu",
  "ubuntu_version": "22.04",
  "pretty_name": "Ubuntu 22.04.5 LTS",
  "l4t_version": "R36.4",
  "jetpack_version": "6.2.1+b38",
  "device_model": "NVIDIA Jetson AGX Orin Developer Kit"
}"#;

    #[tokio::test]
    async fn parses_real_jetson_fixture() {
        let mut ex = MockExec {
            stdout: FIXTURE.as_bytes().to_vec(),
            exit_code: Some(0),
        };
        match detect(&mut ex).await.unwrap() {
            DetectOutcome::Device(d) => {
                assert!(d.is_jetson);
                assert_eq!(d.architecture, "aarch64");
                assert_eq!(d.l4t_version, "R36.4");
                assert_eq!(d.jetpack_version, "6.2.1+b38");
                assert_eq!(d.hostname, "seeed-desktop");
            }
            DetectOutcome::NotJetson => panic!("expected device"),
        }
    }

    #[tokio::test]
    async fn parses_machine_id_serial_and_paths() {
        let mut ex = MockExec {
            stdout: br#"{"is_jetson":true,"hostname":"seeed-desktop","architecture":"aarch64","ubuntu_version":"22.04","l4t_version":"R36.4","jetpack_version":"6.2.1","device_model":"reComputer","machine_id":"5dbfb124","serial_number":"1421123007848","ipv4_addresses":[{"address":"192.168.2.18","kind":"lan"},{"address":"100.114.170.49","kind":"tailscale"}]}"#.to_vec(),
            exit_code: Some(0),
        };
        match detect(&mut ex).await.unwrap() {
            DetectOutcome::Device(d) => {
                assert_eq!(d.machine_id, "5dbfb124");
                assert_eq!(d.serial_number, "1421123007848");
                assert_eq!(d.ipv4_addresses.len(), 2);
                assert_eq!(d.ipv4_addresses[0].address, "192.168.2.18");
                assert_eq!(d.ipv4_addresses[0].kind, "lan");
                assert_eq!(d.ipv4_addresses[1].kind, "tailscale");
            }
            DetectOutcome::NotJetson => panic!("expected device"),
        }

        // Older script output without the new fields defaults gracefully.
        let mut ex = MockExec {
            stdout: FIXTURE.as_bytes().to_vec(),
            exit_code: Some(0),
        };
        match detect(&mut ex).await.unwrap() {
            DetectOutcome::Device(d) => {
                assert!(d.machine_id.is_empty());
                assert!(d.serial_number.is_empty());
                assert!(d.ipv4_addresses.is_empty());
            }
            DetectOutcome::NotJetson => panic!("expected device"),
        }
    }

    #[tokio::test]
    async fn device_id_prefers_serial_over_machine_id() {
        use crate::device::types::JetsonDevice;

        // Cloned vendor images share one machine-id across boards (real
        // finding on the two test J501s); the device-tree serial is unique
        // per module and must win.
        let d: JetsonDetectionResult = serde_json::from_str(
            r#"{"is_jetson":true,"hostname":"seeed-desktop","architecture":"aarch64","ubuntu_version":"22.04","l4t_version":"R36.4","jetpack_version":"6.2.1","device_model":"reComputer","machine_id":"5dbfb124","serial_number":"1421123007848","ipv4_addresses":[]}"#,
        )
        .unwrap();
        let device = JetsonDevice::from_detection("192.168.2.18", d);
        assert_eq!(device.device_id.as_deref(), Some("1421123007848"));

        // No serial → machine-id is the identity.
        let d: JetsonDetectionResult = serde_json::from_str(
            r#"{"is_jetson":true,"hostname":"h","architecture":"aarch64","ubuntu_version":"22.04","l4t_version":"R36.4","jetpack_version":"6.2.1","device_model":"m","machine_id":"5dbfb124","serial_number":"","ipv4_addresses":[]}"#,
        )
        .unwrap();
        let device = JetsonDevice::from_detection("192.168.2.18", d);
        assert_eq!(device.device_id.as_deref(), Some("5dbfb124"));

        // Neither → legacy host-keyed identity (no deviceId).
        let d: JetsonDetectionResult = serde_json::from_str(
            r#"{"is_jetson":true,"hostname":"h","architecture":"aarch64","ubuntu_version":"22.04","l4t_version":"R36.4","jetpack_version":"6.2.1","device_model":"m","ipv4_addresses":[]}"#,
        )
        .unwrap();
        let device = JetsonDevice::from_detection("192.168.2.18", d);
        assert_eq!(device.device_id, None);
    }

    #[tokio::test]
    async fn maps_not_jetson() {
        let mut ex = MockExec {
            stdout: r#"{"is_jetson":false,"hostname":"pi","architecture":"x86_64","ubuntu_version":"22.04","l4t_version":"","jetpack_version":"","device_model":"raspberry"}"#.as_bytes().to_vec(),
            exit_code: Some(0),
        };
        assert!(matches!(
            detect(&mut ex).await.unwrap(),
            DetectOutcome::NotJetson
        ));
    }

    #[tokio::test]
    async fn fails_on_malformed_json() {
        let mut ex = MockExec {
            stdout: b"not json at all".to_vec(),
            exit_code: Some(0),
        };
        assert!(matches!(
            detect(&mut ex).await.unwrap_err(),
            DetectError::Parse
        ));
    }

    #[tokio::test]
    async fn fails_on_nonzero_exit() {
        let mut ex = MockExec {
            stdout: FIXTURE.as_bytes().to_vec(),
            exit_code: Some(2),
        };
        assert!(matches!(
            detect(&mut ex).await.unwrap_err(),
            DetectError::Ssh(SshError::CommandFailed(Some(2)))
        ));
    }
}
