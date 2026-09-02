use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager, State};

use crate::bootstrap::checker;
use crate::bootstrap::error::ProvisionError;
use crate::bootstrap::provisioner::{self, stage_event};
use crate::bootstrap::types::{
    ProvisionEvent, ProvisionStage, RemoteEnvironmentReport, RemoteEnvironmentState,
};
use crate::bootstrap::verifier;
use crate::device::detector::{self, DetectError, DetectOutcome};
use crate::device::types::JetsonDevice;
use crate::remember::{self, FileSecretStore, RememberError, RememberedDeviceStore};
use crate::ssh::client as ssh;
use crate::ssh::error::SshError;
use crate::ssh::types::{HostKeyInfo, SshConfig, SshConnectionInput};
use crate::trust::TrustStoreFile;
use crate::tunnel::{TunnelEndpoints, TunnelError, TunnelManager};

/// User's answer to a host-key prompt. Carries the full key so we can persist
/// algorithm + fingerprint, not just a bare fingerprint.
#[derive(Deserialize)]
#[serde(tag = "action", rename_all = "camelCase")]
pub enum HostKeyDecision {
    TrustAndConnect { key: HostKeyInfo },
    ReplaceAndConnect { key: HostKeyInfo },
}

/// Ok-variant outcomes that require a user decision are NOT errors (§25).
#[derive(Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum ProbeResult {
    Device {
        device: JetsonDevice,
    },
    HostKeyUnknown {
        key: HostKeyInfo,
    },
    HostKeyChanged {
        current: HostKeyInfo,
        previous: HostKeyInfo,
    },
}

#[derive(Serialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
#[allow(dead_code)] // Cancelled reserved for future Rust-side cancellation
pub enum ProbeErrorCode {
    SshTimeout,
    ConnectionRefused,
    AuthenticationFailed,
    SshProtocolError,
    RemoteCommandFailed,
    DetectionParseFailed,
    NotAJetson,
    SudoAuthenticationFailed,
    SudoPermissionDenied,
    ProvisionFailed,
    ProvisionTimeout,
    VerificationFailed,
    Cancelled,
    SavedPasswordMissing,
    Unknown,
}

/// Typed, sanitized IPC error — never a raw `anyhow` debug string (§30).
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProbeError {
    pub code: ProbeErrorCode,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

impl ProbeError {
    pub(crate) fn new(code: ProbeErrorCode, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
            detail: None,
        }
    }
}

/// Map remembered-device failures into typed IPC errors. `Missing` is the only
/// case that needs its own code: the UI asks the user to type a password.
impl From<RememberError> for ProbeError {
    fn from(e: RememberError) -> Self {
        match e {
            RememberError::Missing => ProbeError::new(
                ProbeErrorCode::SavedPasswordMissing,
                "No stored password is available for this device",
            ),
            other => ProbeError::new(ProbeErrorCode::Unknown, other.to_string()),
        }
    }
}

/// Resolve the SSH password: a typed password wins, otherwise fall back to the
/// remembered one in the OS secret store (V0.3 auto-reconnect). The fallback
/// resolves the secret-store account from remembered.json so tunnel mode
/// (wire host rewritten to loopback, KI-021) still finds the stored secret.
fn resolve_ssh_password(
    app: &AppHandle,
    secrets: &FileSecretStore,
    input: &SshConnectionInput,
) -> Result<String, ProbeError> {
    let store = RememberedDeviceStore::for_app(app)
        .map_err(|e| ProbeError::new(ProbeErrorCode::Unknown, format!("config dir: {e}")))?;
    remember::resolve_password(
        &store,
        secrets,
        &input.host,
        &input.username,
        input.password.as_deref(),
    )
    .map_err(ProbeError::from)
}

#[tauri::command]
pub async fn probe_device(
    app: AppHandle,
    secrets: State<'_, FileSecretStore>,
    tunnels: State<'_, TunnelManager>,
    input: SshConnectionInput,
    host_key_decision: Option<HostKeyDecision>,
) -> Result<ProbeResult, ProbeError> {
    eprintln!(
        "[jr-flow] probe start host={} port={} user={}",
        input.host, input.port, input.username
    );
    let password = resolve_ssh_password(&app, &secrets, &input)?;
    // The app carries its own loopback tunnel (system ssh, KI-021); both
    // planes connect to the returned endpoints instead of the LAN host.
    let endpoints = ensure_tunnel(&tunnels, &app, &input, &password).await?;
    let mut wire = input.clone();
    wire.host = endpoints.host.clone();
    wire.port = endpoints.ssh_port;
    let config = SshConfig::default();
    let config_dir = app
        .path()
        .app_config_dir()
        .map_err(|e| ProbeError::new(ProbeErrorCode::Unknown, format!("config dir: {e}")))?;
    let mut store = TrustStoreFile::load(config_dir)
        .map_err(|e| ProbeError::new(ProbeErrorCode::Unknown, format!("trust store: {e}")))?;

    // 1. Persist a trust/replace decision before reconnecting.
    if let Some(decision) = &host_key_decision {
        let key = match decision {
            HostKeyDecision::TrustAndConnect { key }
            | HostKeyDecision::ReplaceAndConnect { key } => key,
        };
        store
            .save(key)
            .map_err(|e| ProbeError::new(ProbeErrorCode::Unknown, format!("save trust: {e}")))?;
    }

    // 2. Ephemeral connect with TOFU host-key verification (wire endpoint).
    eprintln!("[jr-flow] probe connecting...");
    let expected = store.get_fingerprint(&wire.host, wire.port);
    let mut session = match ssh::connect(&wire, expected.as_deref(), &config).await {
        Ok(ssh::SshConnectOutcome::Connected(s)) => s,
        Ok(ssh::SshConnectOutcome::HostKeyUnknown(key)) => {
            return Ok(ProbeResult::HostKeyUnknown { key });
        }
        Ok(ssh::SshConnectOutcome::HostKeyChanged { current, expected }) => {
            let previous = store
                .get(&wire.host, wire.port)
                .map(|h| HostKeyInfo {
                    host: wire.host.clone(),
                    port: wire.port,
                    algorithm: h.algorithm,
                    fingerprint: h.fingerprint,
                })
                .unwrap_or_else(|| HostKeyInfo {
                    host: wire.host.clone(),
                    port: wire.port,
                    algorithm: "unknown".into(),
                    fingerprint: expected,
                });
            return Ok(ProbeResult::HostKeyChanged { current, previous });
        }
        Err(e) => return Err(map_ssh_error(e)),
    };

    // 3. Authenticate.
    session
        .authenticate_password(&wire.username, &password)
        .await
        .map_err(map_ssh_error)?;
    eprintln!("[jr-flow] probe authenticated");

    // 4. Detect (device identity keeps the LAN host the user typed).
    let outcome = detector::detect(&mut session)
        .await
        .map_err(map_detect_error)?;
    eprintln!("[jr-flow] probe detected ok");
    let device = match outcome {
        DetectOutcome::Device(d) => JetsonDevice::from_detection(&input.host, d),
        DetectOutcome::NotJetson => {
            return Err(ProbeError::new(
                ProbeErrorCode::NotAJetson,
                "Not an NVIDIA Jetson",
            ));
        }
    };

    Ok(ProbeResult::Device { device })
}

fn map_ssh_error(e: SshError) -> ProbeError {
    match e {
        SshError::Timeout => ProbeError::new(ProbeErrorCode::SshTimeout, "SSH connect timed out"),
        SshError::Connect(_) => ProbeError::new(
            ProbeErrorCode::ConnectionRefused,
            "Could not reach the device",
        ),
        SshError::AuthRejected => ProbeError::new(
            ProbeErrorCode::AuthenticationFailed,
            "Authentication failed",
        ),
        SshError::CommandFailed(code) => ProbeError::new(
            ProbeErrorCode::RemoteCommandFailed,
            format!("Remote command failed (exit {code:?})"),
        ),
        SshError::OutputTooLarge => ProbeError::new(
            ProbeErrorCode::RemoteCommandFailed,
            "Remote output too large",
        ),
    }
}

fn map_detect_error(e: DetectError) -> ProbeError {
    match e {
        DetectError::Ssh(e) => map_ssh_error(e),
        DetectError::Parse => ProbeError::new(
            ProbeErrorCode::DetectionParseFailed,
            "Could not read device information",
        ),
    }
}

fn map_tunnel_error(e: TunnelError) -> ProbeError {
    match e {
        TunnelError::AuthFailed => ProbeError::new(
            ProbeErrorCode::AuthenticationFailed,
            "Authentication failed",
        ),
        TunnelError::Unreachable => {
            ProbeError::new(ProbeErrorCode::SshTimeout, "Could not reach the device")
        }
        TunnelError::Setup(m) => {
            ProbeError::new(ProbeErrorCode::Unknown, format!("Secure tunnel: {m}"))
        }
    }
}

/// Establish (or reuse) the in-app loopback tunnel off the async runtime —
/// spawning ssh + polling ports is blocking work.
async fn ensure_tunnel(
    tunnels: &TunnelManager,
    app: &AppHandle,
    input: &SshConnectionInput,
    password: &str,
) -> Result<TunnelEndpoints, ProbeError> {
    let manager = tunnels.clone();
    let app = app.clone();
    let host = input.host.clone();
    let username = input.username.clone();
    let remote_port = input.port;
    let password = password.to_string();
    tokio::task::spawn_blocking(move || {
        manager.ensure(&app, &host, remote_port, &username, &password)
    })
    .await
    .map_err(|e| ProbeError::new(ProbeErrorCode::Unknown, format!("tunnel task: {e}")))?
    .map_err(map_tunnel_error)
}

#[derive(Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum PrepareResult {
    #[serde(rename_all = "camelCase")]
    Ready {
        was_already_ready: bool,
        environment: RemoteEnvironmentReport,
    },
    HostKeyUnknown {
        key: HostKeyInfo,
    },
    HostKeyChanged {
        current: HostKeyInfo,
        previous: HostKeyInfo,
    },
}

/// Check the remote desktop environment, provisioning it if necessary, and
/// verify it is Ready. Streams `ProvisionEvent`s over a Tauri IPC channel.
#[tauri::command]
pub async fn prepare_remote_desktop(
    app: AppHandle,
    secrets: State<'_, FileSecretStore>,
    tunnels: State<'_, TunnelManager>,
    input: SshConnectionInput,
    host_key_decision: Option<HostKeyDecision>,
    on_event: tauri::ipc::Channel<ProvisionEvent>,
) -> Result<PrepareResult, ProbeError> {
    eprintln!(
        "[jr-flow] prepare start host={} port={} user={}",
        input.host, input.port, input.username
    );
    let password = resolve_ssh_password(&app, &secrets, &input)?;
    let endpoints = ensure_tunnel(&tunnels, &app, &input, &password).await?;
    let mut wire = input.clone();
    wire.host = endpoints.host.clone();
    wire.port = endpoints.ssh_port;
    let config = SshConfig::default();
    let config_dir = app
        .path()
        .app_config_dir()
        .map_err(|e| ProbeError::new(ProbeErrorCode::Unknown, format!("config dir: {e}")))?;
    let mut store = TrustStoreFile::load(config_dir)
        .map_err(|e| ProbeError::new(ProbeErrorCode::Unknown, format!("trust store: {e}")))?;

    if let Some(decision) = &host_key_decision {
        let key = match decision {
            HostKeyDecision::TrustAndConnect { key }
            | HostKeyDecision::ReplaceAndConnect { key } => key,
        };
        store
            .save(key)
            .map_err(|e| ProbeError::new(ProbeErrorCode::Unknown, format!("save trust: {e}")))?;
    }

    let expected = store.get_fingerprint(&wire.host, wire.port);
    let mut session = match ssh::connect(&wire, expected.as_deref(), &config).await {
        Ok(ssh::SshConnectOutcome::Connected(s)) => s,
        Ok(ssh::SshConnectOutcome::HostKeyUnknown(key)) => {
            return Ok(PrepareResult::HostKeyUnknown { key });
        }
        Ok(ssh::SshConnectOutcome::HostKeyChanged { current, expected }) => {
            let previous = store
                .get(&wire.host, wire.port)
                .map(|h| HostKeyInfo {
                    host: wire.host.clone(),
                    port: wire.port,
                    algorithm: h.algorithm,
                    fingerprint: h.fingerprint,
                })
                .unwrap_or_else(|| HostKeyInfo {
                    host: wire.host.clone(),
                    port: wire.port,
                    algorithm: "unknown".into(),
                    fingerprint: expected,
                });
            return Ok(PrepareResult::HostKeyChanged { current, previous });
        }
        Err(e) => return Err(map_ssh_error(e)),
    };

    session
        .authenticate_password(&wire.username, &password)
        .await
        .map_err(map_ssh_error)?;

    let _ = on_event.send(stage_event(ProvisionStage::CheckingEnvironment));
    let mut report = checker::check(&mut session)
        .await
        .map_err(map_provision_error)?;

    if report.state == RemoteEnvironmentState::Ready {
        let _ = on_event.send(stage_event(ProvisionStage::AlreadyReady));
        return Ok(PrepareResult::Ready {
            was_already_ready: true,
            environment: report,
        });
    }

    let _ = on_event.send(stage_event(ProvisionStage::ProvisionRequired));
    let channel = on_event.clone();
    provisioner::provision(&mut session, &password, move |ev| {
        let _ = channel.send(ev);
    })
    .await
    .map_err(map_provision_error)?;

    let _ = on_event.send(stage_event(ProvisionStage::Verifying));
    report = verifier::verify(&mut session)
        .await
        .map_err(map_provision_error)?;
    let _ = on_event.send(stage_event(ProvisionStage::Complete));

    Ok(PrepareResult::Ready {
        was_already_ready: false,
        environment: report,
    })
}

fn map_provision_error(e: ProvisionError) -> ProbeError {
    match e {
        ProvisionError::Ssh(e) => map_ssh_error(e),
        ProvisionError::CheckParse => ProbeError::new(
            ProbeErrorCode::DetectionParseFailed,
            "Could not read environment",
        ),
        ProvisionError::SudoAuthFailed => ProbeError::new(
            ProbeErrorCode::SudoAuthenticationFailed,
            "Administrator access is required",
        ),
        ProvisionError::SudoPermissionDenied => ProbeError::new(
            ProbeErrorCode::SudoPermissionDenied,
            "Administrator access is required (user not allowed to sudo)",
        ),
        ProvisionError::TempFile => ProbeError::new(
            ProbeErrorCode::ProvisionFailed,
            "Could not prepare the setup script",
        ),
        ProvisionError::BootstrapFailed(code) => ProbeError::new(
            ProbeErrorCode::ProvisionFailed,
            format!("Remote desktop setup failed (exit {code:?})"),
        ),
        ProvisionError::BootstrapTimeout => ProbeError::new(
            ProbeErrorCode::ProvisionTimeout,
            "Remote desktop setup timed out",
        ),
        ProvisionError::VerificationFailed => ProbeError::new(
            ProbeErrorCode::VerificationFailed,
            "Remote desktop did not become ready",
        ),
    }
}
