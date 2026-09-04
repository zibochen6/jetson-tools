use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RemoteEnvironmentState {
    /// Fully installed, configured, running — nothing to do.
    Ready,
    /// Some components installed but the full stack is not (repair path).
    Partial,
    /// Components present but the service/session is broken (repair path).
    Broken,
    /// Nothing installed yet — needs a fresh provision.
    ProvisionRequired,
}

/// Facts gathered by `scripts/remote/check-environment.sh` (read-only).
/// Keys are snake_case to exactly mirror the shell script's JSON contract.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct EnvironmentFacts {
    pub xrdp_installed: bool,
    #[serde(default)]
    pub xrdp_version: String,
    pub xorgxrdp_installed: bool,
    #[serde(default)]
    pub xorgxrdp_version: String,
    pub xfce_installed: bool,
    pub xrdp_enabled: bool,
    pub xrdp_active: bool,
    #[serde(default)]
    pub xrdp_sesman_active: bool,
    pub port_3389_listening: bool,
    #[serde(default)]
    pub port_3350_listening: bool,
    /// The stock Ubuntu XRDP key is readable only when `xrdp` belongs to
    /// `ssl-cert`; without this, TCP succeeds but every TLS handshake fails.
    #[serde(default)]
    pub xrdp_in_ssl_cert_group: bool,
    pub session_configured: bool,
    pub xsessionrc_ok: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RemoteEnvironmentReport {
    pub state: RemoteEnvironmentState,
    #[serde(flatten)]
    pub facts: EnvironmentFacts,
    pub issues: Vec<String>,
}

/// Classify raw environment facts into a state + human-readable issues.
/// Pure and unit-testable — the shell returns facts, Rust decides state.
pub fn classify(facts: &EnvironmentFacts) -> RemoteEnvironmentReport {
    let mut issues: Vec<String> = Vec::new();
    if !facts.xrdp_installed {
        issues.push("xrdp is not installed".into());
    }
    if !facts.xorgxrdp_installed {
        issues.push("xorgxrdp is not installed".into());
    }
    if !facts.xfce_installed {
        issues.push("xfce is not installed".into());
    }
    if !facts.xrdp_enabled {
        issues.push("xrdp is not enabled (systemctl)".into());
    }
    if !facts.xrdp_active {
        issues.push("xrdp service is not running".into());
    }
    if !facts.xrdp_sesman_active {
        issues.push("xrdp-sesman service is not running".into());
    }
    if !facts.port_3389_listening {
        issues.push("port 3389 is not listening".into());
    }
    if !facts.port_3350_listening {
        issues.push("xrdp-sesman port 3350 is not listening".into());
    }
    if !facts.xrdp_in_ssl_cert_group {
        issues.push("xrdp cannot read its TLS key (missing ssl-cert group membership)".into());
    }
    if !facts.session_configured {
        issues.push("session is not configured (~/.xsession)".into());
    }
    if !facts.xsessionrc_ok {
        issues.push(".xsessionrc has a shell syntax error".into());
    }

    let any_installed = facts.xrdp_installed || facts.xorgxrdp_installed || facts.xfce_installed;
    let all_components = facts.xrdp_installed && facts.xorgxrdp_installed && facts.xfce_installed;
    let service_ok = facts.xrdp_enabled
        && facts.xrdp_active
        && facts.xrdp_sesman_active
        && facts.port_3389_listening
        && facts.port_3350_listening
        && facts.xrdp_in_ssl_cert_group;

    let state = if all_components && service_ok && facts.session_configured && facts.xsessionrc_ok {
        RemoteEnvironmentState::Ready
    } else if !any_installed {
        RemoteEnvironmentState::ProvisionRequired
    } else if !all_components {
        RemoteEnvironmentState::Partial
    } else {
        RemoteEnvironmentState::Broken
    };

    RemoteEnvironmentReport {
        state,
        facts: facts.clone(),
        issues,
    }
}

/// Provision lifecycle stages, streamed to the UI in real time.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ProvisionStage {
    CheckingEnvironment,
    AlreadyReady,
    ProvisionRequired,
    Preflight,
    Uploading,
    InstallingPackages,
    ConfiguringSession,
    StartingService,
    Verifying,
    Complete,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProvisionEvent {
    pub stage: ProvisionStage,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub progress: Option<f64>,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn facts(overrides: impl Fn(&mut EnvironmentFacts)) -> EnvironmentFacts {
        let mut f = EnvironmentFacts {
            xrdp_installed: true,
            xrdp_version: "0.9.17".into(),
            xorgxrdp_installed: true,
            xorgxrdp_version: "0.2.17".into(),
            xfce_installed: true,
            xrdp_enabled: true,
            xrdp_active: true,
            xrdp_sesman_active: true,
            port_3389_listening: true,
            port_3350_listening: true,
            xrdp_in_ssl_cert_group: true,
            session_configured: true,
            xsessionrc_ok: true,
        };
        overrides(&mut f);
        f
    }

    #[test]
    fn classifies_ready() {
        assert_eq!(
            classify(&facts(|_| {})).state,
            RemoteEnvironmentState::Ready
        );
    }

    #[test]
    fn classifies_provision_required_when_nothing_installed() {
        let f = facts(|f| {
            f.xrdp_installed = false;
            f.xorgxrdp_installed = false;
            f.xfce_installed = false;
        });
        assert_eq!(
            classify(&f).state,
            RemoteEnvironmentState::ProvisionRequired
        );
    }

    #[test]
    fn classifies_partial_when_a_component_missing() {
        let f = facts(|f| f.xorgxrdp_installed = false);
        assert_eq!(classify(&f).state, RemoteEnvironmentState::Partial);
    }

    #[test]
    fn classifies_broken_when_service_stopped() {
        let f = facts(|f| f.xrdp_active = false);
        assert_eq!(classify(&f).state, RemoteEnvironmentState::Broken);
    }

    #[test]
    fn classifies_broken_when_sesman_is_unavailable() {
        let f = facts(|f| {
            f.xrdp_sesman_active = false;
            f.port_3350_listening = false;
        });
        let report = classify(&f);
        assert_eq!(report.state, RemoteEnvironmentState::Broken);
        assert!(report.issues.iter().any(|issue| issue.contains("sesman")));
    }

    #[test]
    fn classifies_broken_when_xsessionrc_broken() {
        let f = facts(|f| f.xsessionrc_ok = false);
        assert_eq!(classify(&f).state, RemoteEnvironmentState::Broken);
    }

    #[test]
    fn classifies_broken_when_xrdp_cannot_read_its_tls_key() {
        let f = facts(|f| f.xrdp_in_ssl_cert_group = false);
        let report = classify(&f);
        assert_eq!(report.state, RemoteEnvironmentState::Broken);
        assert!(report.issues.iter().any(|issue| issue.contains("ssl-cert")));
    }
}
