use thiserror::Error;

use crate::ssh::error::SshError;

/// Provisioning-layer errors, mapped to typed `ProbeErrorCode` at the IPC
/// boundary. Secrets are never embedded in these messages.
#[derive(Debug, Error)]
pub enum ProvisionError {
    #[error(transparent)]
    Ssh(#[from] SshError),
    #[error("environment check output was not valid JSON")]
    CheckParse,
    #[error("administrator access is required (sudo authentication failed)")]
    SudoAuthFailed,
    #[error("administrator access is required (user not allowed to sudo)")]
    SudoPermissionDenied,
    #[error("could not prepare the temporary script on the device")]
    TempFile,
    #[error("remote desktop setup failed (exit code {0:?})")]
    BootstrapFailed(Option<u32>),
    #[error("remote desktop setup timed out")]
    BootstrapTimeout,
    #[error("remote desktop did not become ready after setup")]
    VerificationFailed,
}
