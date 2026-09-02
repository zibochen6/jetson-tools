use thiserror::Error;

/// Internal SSH-layer error. Mapped to the typed `ProbeErrorCode` at the IPC
/// boundary — never surfaced to the frontend raw.
#[derive(Debug, Error)]
pub enum SshError {
    #[error("ssh connection failed")]
    Connect(#[from] russh::Error),
    #[error("ssh connect timed out")]
    Timeout,
    #[error("authentication rejected")]
    AuthRejected,
    #[error("remote command failed (exit code {0:?})")]
    CommandFailed(Option<u32>),
    #[error("remote output exceeded limit")]
    OutputTooLarge,
}
