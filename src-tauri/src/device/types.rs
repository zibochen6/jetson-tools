use serde::{Deserialize, Serialize};

/// One reachable address of a device, classified by network kind.
/// `lan` = ordinary private network, `tailscale` = 100.64.0.0/10 (CGNAT).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DevicePath {
    pub kind: String,
    pub address: String,
}

impl DevicePath {
    pub fn lan(address: impl Into<String>) -> Self {
        Self {
            kind: "lan".into(),
            address: address.into(),
        }
    }
}

/// Mirrors the JSON contract emitted by `scripts/remote/detect.sh`.
/// Unknown fields are ignored; `machine_id` / `ipv4_addresses` default when
/// an older remote script version is embedded.
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
    /// Stable device identity candidates from detect.sh.
    /// REAL-DEVICE FINDING: cloned vendor images share one `/etc/machine-id`
    /// across boards (both test J501s reported the same value), so the
    /// device-tree serial number — unique per Jetson module — is the primary
    /// identity and machine-id is only the fallback.
    #[serde(default)]
    pub serial_number: String,
    #[serde(default)]
    pub machine_id: String,
    /// The device's current IPv4 addresses (loopback/docker/USB/public
    /// filtered by detect.sh).
    #[serde(default)]
    pub ipv4_addresses: Vec<DetectedIp>,
}

/// One row of detect.sh's `ipv4_addresses`.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct DetectedIp {
    pub address: String,
    pub kind: String,
}

/// Product device model surfaced to the frontend.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct JetsonDevice {
    /// The entry address the user typed this round (may be any path).
    pub host: String,
    /// Stable identity (`/etc/machine-id`); `None` when the device has none.
    pub device_id: Option<String>,
    /// The device's current candidate paths, as reported by detect.sh.
    pub paths: Vec<DevicePath>,
    pub hostname: Option<String>,
    pub model: Option<String>,
    pub architecture: Option<String>,
    pub ubuntu_version: Option<String>,
    pub jetpack_version: Option<String>,
    pub l4t_version: Option<String>,
}

impl JetsonDevice {
    pub fn from_detection(host: &str, d: JetsonDetectionResult) -> Self {
        // Identity precedence: device-tree serial (unique per module, survives
        // OS reinstalls) → machine-id (cloned images can share it) → None
        // (legacy host-keyed identity).
        let device_id = [d.serial_number.trim(), d.machine_id.trim()]
            .into_iter()
            .find(|s| !s.is_empty())
            .map(str::to_string);
        let paths = d
            .ipv4_addresses
            .into_iter()
            .map(|ip| DevicePath {
                kind: ip.kind,
                address: ip.address,
            })
            .collect();
        Self {
            host: host.to_string(),
            device_id,
            paths,
            hostname: Some(d.hostname),
            model: Some(d.device_model),
            architecture: Some(d.architecture),
            ubuntu_version: Some(d.ubuntu_version),
            jetpack_version: Some(d.jetpack_version),
            l4t_version: Some(d.l4t_version),
        }
    }
}
