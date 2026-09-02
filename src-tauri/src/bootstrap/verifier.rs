use crate::ssh::executor::RemoteExecutor;

use super::checker::check;
use super::error::ProvisionError;
use super::types::{RemoteEnvironmentReport, RemoteEnvironmentState};

/// Post-provision re-check. A bootstrap that returned exit 0 does NOT prove
/// readiness — this verifies the environment is actually Now Ready.
pub async fn verify<E: RemoteExecutor>(
    executor: &mut E,
) -> Result<RemoteEnvironmentReport, ProvisionError> {
    let report = check(executor).await?;
    if report.state != RemoteEnvironmentState::Ready {
        return Err(ProvisionError::VerificationFailed);
    }
    Ok(report)
}
