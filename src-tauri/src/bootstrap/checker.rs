use crate::ssh::executor::RemoteExecutor;

use super::error::ProvisionError;
use super::types::{classify, EnvironmentFacts, RemoteEnvironmentReport};

/// `check-environment.sh` embedded at compile time (same pattern as detect.sh).
pub const CHECK_SCRIPT: &str = include_str!("../../../scripts/remote/check-environment.sh");

/// Run the read-only environment probe and classify the result.
pub async fn check<E: RemoteExecutor>(
    executor: &mut E,
) -> Result<RemoteEnvironmentReport, ProvisionError> {
    let result = executor
        .exec_with_stdin("sh -s", CHECK_SCRIPT.as_bytes())
        .await?;

    if let Some(code) = result.exit_code {
        if code != 0 {
            return Err(ProvisionError::Ssh(
                crate::ssh::error::SshError::CommandFailed(Some(code)),
            ));
        }
    }

    let stdout = String::from_utf8_lossy(&result.stdout);
    let facts: EnvironmentFacts =
        serde_json::from_str(stdout.trim()).map_err(|_| ProvisionError::CheckParse)?;

    Ok(classify(&facts))
}
