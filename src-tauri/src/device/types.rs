use serde::{Deserialize, Serialize};

/// Mirrors the JSON contract emitted by `scripts/remote/detect.sh`.
/// Unknown fields (remote_desktop, nv_tegra_release, pretty_name, …) are ignored.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct JetsonDetectionResult {
    pub is_jetson: bool,
    pub hostname: String,
    pub architecture: String,
    pub ubuntu_version: String,
    pub l4t_version: String,
    pub jetpack_version: String,
    pub device_model: String,
}

/// Product device model surfaced to the frontend (matches Phase 1's shape).
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct JetsonDevice {
    pub host: String,
    pub hostname: Option<String>,
    pub model: Option<String>,
    pub architecture: Option<String>,
    pub ubuntu_version: Option<String>,
    pub jetpack_version: Option<String>,
    pub l4t_version: Option<String>,
}

impl JetsonDevice {
    pub fn from_detection(host: &str, d: JetsonDetectionResult) -> Self {
        Self {
            host: host.to_string(),
            hostname: Some(d.hostname),
            model: Some(d.device_model),
            architecture: Some(d.architecture),
            ubuntu_version: Some(d.ubuntu_version),
            jetpack_version: Some(d.jetpack_version),
            l4t_version: Some(d.l4t_version),
        }
    }
}
