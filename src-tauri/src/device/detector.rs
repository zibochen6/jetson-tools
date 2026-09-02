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
