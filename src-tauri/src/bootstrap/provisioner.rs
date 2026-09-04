use crate::ssh::executor::RemoteExecutor;

use super::error::ProvisionError;
use super::types::{ProvisionEvent, ProvisionStage};

/// `bootstrap.sh` embedded at compile time (Phase 0 verified, idempotent,
/// includes the ~/.xsessionrc compatibility fix).
pub const BOOTSTRAP_SCRIPT: &str = include_str!("../../../scripts/remote/bootstrap.sh");

pub fn stage_message(stage: ProvisionStage) -> &'static str {
    match stage {
        ProvisionStage::CheckingEnvironment => "Checking remote desktop",
        ProvisionStage::AlreadyReady => "Remote desktop ready",
        ProvisionStage::ProvisionRequired => "Remote desktop needs setup",
        ProvisionStage::Preflight => "Checking administrator access",
        ProvisionStage::Uploading => "Uploading setup",
        ProvisionStage::InstallingPackages => "Installing remote desktop components",
        ProvisionStage::ConfiguringSession => "Configuring the desktop",
        ProvisionStage::StartingService => "Starting remote desktop",
        ProvisionStage::Verifying => "Verifying setup",
        ProvisionStage::Complete => "Setup complete",
    }
}

/// `sudo_preflight` has already validated and cached authorization. The
/// bootstrap command must therefore use that ticket non-interactively instead
/// of receiving the password a second time on stdin: sudo may skip reading it,
/// leaving the secret to be interpreted as the first line of the shell script.
fn bootstrap_command(path: &str) -> String {
    format!("sudo -n bash '{path}'")
}

pub fn stage_event(stage: ProvisionStage) -> ProvisionEvent {
    ProvisionEvent {
        stage,
        message: stage_message(stage).to_string(),
        detail: None,
        progress: None,
    }
}

/// Parse one bootstrap stdout line into a ProvisionEvent, if it is a machine
/// marker (`[bootstrap] step=<name>`). Natural-language lines are ignored.
pub fn parse_bootstrap_line(line: &str) -> Option<ProvisionEvent> {
    let line = line.trim();
    let step = line.strip_prefix("[bootstrap] step=")?;
    let step = step.split_whitespace().next()?;
    let stage = match step {
        "install_packages" => ProvisionStage::InstallingPackages,
        "configure_session" | "fix_xsessionrc" | "disable_wayland" => {
            ProvisionStage::ConfiguringSession
        }
        "enable_service" => ProvisionStage::StartingService,
        "verify" => ProvisionStage::Verifying,
        _ => return None,
    };
    Some(stage_event(stage))
}

/// Verify sudo works (and the password is correct) before any system mutation.
/// Distinguishes "wrong password" from "not allowed to sudo" via stderr.
pub async fn sudo_preflight<E: RemoteExecutor>(
    executor: &mut E,
    password: &str,
) -> Result<(), ProvisionError> {
    let result = executor
        .exec_with_stdin("sudo -S -p '' -v", format!("{password}\n").as_bytes())
        .await?;
    match result.exit_code {
        Some(0) | None => Ok(()),
        Some(_) => {
            let stderr = String::from_utf8_lossy(&result.stderr).to_lowercase();
            if stderr.contains("sudoers") {
                Err(ProvisionError::SudoPermissionDenied)
            } else {
                Err(ProvisionError::SudoAuthFailed)
            }
        }
    }
}

/// Provision the remote desktop using the verified bootstrap script:
/// preflight → upload to a safe temp file → run (streaming progress) → cleanup.
/// The password travels only through the SSH channel stdin — never argv/logs.
pub async fn provision<E, F>(
    executor: &mut E,
    password: &str,
    mut emit: F,
) -> Result<(), ProvisionError>
where
    E: RemoteExecutor,
    F: FnMut(ProvisionEvent) + Send,
{
    emit(stage_event(ProvisionStage::Preflight));
    sudo_preflight(executor, password).await?;

    emit(stage_event(ProvisionStage::Uploading));

    // Secure, unpredictable temp path (no predictable-path race).
    let mktemp = executor
        .exec("mktemp /tmp/jetson-remote-bootstrap-XXXXXX.sh")
        .await?;
    let path = String::from_utf8_lossy(&mktemp.stdout).trim().to_string();
    if path.is_empty() || !path.starts_with('/') {
        return Err(ProvisionError::TempFile);
    }

    // Upload + run, guaranteeing cleanup on every exit path.
    let run = provision_inner(executor, &path, &mut emit).await;
    let _ = executor.exec(&format!("rm -f '{path}'")).await;
    run
}

async fn provision_inner<E, F>(
    executor: &mut E,
    path: &str,
    emit: &mut F,
) -> Result<(), ProvisionError>
where
    E: RemoteExecutor,
    F: FnMut(ProvisionEvent) + Send,
{
    executor
        .exec_with_stdin(&format!("cat > '{path}'"), BOOTSTRAP_SCRIPT.as_bytes())
        .await?;
    executor.exec(&format!("chmod +x '{path}'")).await?;

    let mut last_stage: Option<ProvisionStage> = None;
    let result = executor
        .exec_with_stdin_lines(&bootstrap_command(path), &[], |line| {
            if let Some(event) = parse_bootstrap_line(line) {
                // Dedupe consecutive identical stages (start + done markers).
                if last_stage != Some(event.stage) {
                    last_stage = Some(event.stage);
                    emit(event);
                }
            }
        })
        .await?;

    match result.exit_code {
        Some(0) | None => Ok(()),
        Some(code) => Err(ProvisionError::BootstrapFailed(Some(code))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ssh::error::SshError;
    use crate::ssh::executor::{RemoteCommandResult, RemoteExecutor};

    struct ScriptedExec {
        // Responses consumed in order for exec-like calls.
        steps: Vec<RemoteCommandResult>,
        line_output: Vec<String>,
    }

    impl RemoteExecutor for ScriptedExec {
        async fn exec(&mut self, _cmd: &str) -> Result<RemoteCommandResult, SshError> {
            Ok(self.steps.remove(0))
        }
        async fn exec_with_stdin(
            &mut self,
            _cmd: &str,
            _stdin: &[u8],
        ) -> Result<RemoteCommandResult, SshError> {
            Ok(self.steps.remove(0))
        }
        async fn exec_with_stdin_lines<G: FnMut(&str) + Send>(
            &mut self,
            _cmd: &str,
            _stdin: &[u8],
            mut on_line: G,
        ) -> Result<RemoteCommandResult, SshError> {
            for l in self.line_output.drain(..) {
                on_line(&l);
            }
            Ok(self.steps.remove(0))
        }
    }

    fn ok_result() -> RemoteCommandResult {
        RemoteCommandResult {
            stdout: Vec::new(),
            stderr: Vec::new(),
            exit_code: Some(0),
        }
    }

    #[test]
    fn parses_machine_markers_not_natural_language() {
        assert_eq!(
            parse_bootstrap_line("[bootstrap] step=install_packages status=start")
                .unwrap()
                .stage,
            ProvisionStage::InstallingPackages
        );
        assert_eq!(
            parse_bootstrap_line("[bootstrap] step=enable_service status=done")
                .unwrap()
                .stage,
            ProvisionStage::StartingService
        );
        assert!(parse_bootstrap_line("Reading package lists...").is_none());
        assert!(parse_bootstrap_line("[bootstrap] phase=start user=seeed").is_none());
    }

    #[tokio::test]
    async fn sudo_preflight_maps_permission_denied() {
        let mut ex = ScriptedExec {
            steps: vec![RemoteCommandResult {
                stdout: vec![],
                stderr: b"user is not in the sudoers file".to_vec(),
                exit_code: Some(1),
            }],
            line_output: vec![],
        };
        let err = sudo_preflight(&mut ex, "pw").await.unwrap_err();
        assert!(matches!(err, ProvisionError::SudoPermissionDenied));
    }

    #[tokio::test]
    async fn provision_runs_bootstrap_and_returns_ok() {
        // Consumed in order: preflight, mktemp, cat, chmod, bootstrap run, rm.
        let mut ex = ScriptedExec {
            steps: vec![
                ok_result(), // sudo preflight
                RemoteCommandResult {
                    stdout: b"/tmp/jr-boot-ABC123.sh\n".to_vec(),
                    stderr: vec![],
                    exit_code: Some(0),
                },
                ok_result(), // cat upload
                ok_result(), // chmod
                ok_result(), // bootstrap run
                ok_result(), // rm -f cleanup
            ],
            line_output: vec!["[bootstrap] step=install_packages status=start".into()],
        };
        let mut events = vec![];
        provision(&mut ex, "pw", |e| events.push(e.stage))
            .await
            .unwrap();
        assert!(events.contains(&ProvisionStage::Preflight));
        assert!(events.contains(&ProvisionStage::InstallingPackages));
    }

    #[test]
    fn emitted_messages_never_contain_secret() {
        // All stage messages are static product copy; assert none embed a
        // marker "password" token that could leak via progress UI.
        let stages = [
            ProvisionStage::Preflight,
            ProvisionStage::Uploading,
            ProvisionStage::InstallingPackages,
        ];
        for s in stages {
            let ev = stage_event(s);
            assert!(!ev.message.contains("SUPERSECRET"));
            assert!(!ev.message.contains("pw"));
        }
    }

    #[test]
    fn bootstrap_reuses_preflight_authorization_without_replaying_password() {
        assert_eq!(
            bootstrap_command("/tmp/jetson-remote-bootstrap-ABC123.sh"),
            "sudo -n bash '/tmp/jetson-remote-bootstrap-ABC123.sh'"
        );
    }
}
