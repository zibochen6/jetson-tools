// Connection domain types shared across the app.
//
// The state machine is the single source of truth for "where are we now";
// UI must not derive connectivity from a pile of booleans.

/** Full product lifecycle state (Phase 2 implements detect + trust prompts). */
export type ConnectionState =
  | "idle"
  | "connecting_ssh"
  | "authenticating"
  | "detecting_device"
  | "checking_environment"
  | "provision_required"
  | "provisioning"
  | "verifying"
  | "ready"
  | "creating_tunnel"
  | "launching_rdp"
  | "connected"
  | "desktop_opened"
  | "disconnected"
  | "naming_device"
  | "host_key_unknown"
  | "host_key_changed"
  | "error";

/** Machine-readable error codes; mapped to human copy via `describeError`. */
export type ConnectionErrorCode =
  | "ssh_timeout"
  | "auth_failed"
  | "not_jetson"
  | "provision_failed"
  | "rdp_failed"
  | "rdp_client_missing"
  | "rdp_connection_failed"
  | "detection_failed"
  | "sudo_required"
  | "verification_failed"
  | "saved_password_missing"
  | "unknown";

/**
 * Device metadata. Deliberately has NO `password` field — credentials are
 * kept separate from device identity (see PRD §29 / ARCHITECTURE §7).
 *
 * Identity model (v3): one device = one stable id (device-tree serial,
 * machine-id fallback) + a required display name + a mutable path list
 * (LAN / Tailscale). The host the user typed is just this round's entry
 * point.
 */
export interface JetsonDevice {
  /** The entry address used for this round's connection. */
  host: string;
  /**
   * Stable identity: the device-tree serial number (unique per Jetson
   * module), falling back to `/etc/machine-id`; null when the device has
   * neither. Real-device finding: cloned vendor images SHARE one machine-id
   * across boards, so the serial is the primary identity.
   */
  deviceId?: string | null;
  /** The device's current candidate addresses, as reported by detect.sh. */
  paths?: DevicePath[];
  hostname?: string;
  model?: string;
  architecture?: string;
  ubuntuVersion?: string;
  jetpackVersion?: string;
  l4tVersion?: string;
}

/** One candidate address of a device, classified by network kind. */
export interface DevicePath {
  kind: "lan" | "tailscale" | string;
  address: string;
}

/** Progress emitted during a connection: system state + display text. */
export interface ConnectionProgress {
  state: ConnectionState;
  message: string;
  detail?: string;
  /** Optional 0..1 progress when the underlying step is quantifiable. */
  progress?: number;
}

/** User-supplied connection input. Password is transient, memory-only. */
export interface ConnectionInput {
  host: string;
  username: string;
  password: string;
  remember: boolean;
  /** Stable device identity when connecting a remembered v3 device. */
  deviceId?: string | null;
}

export interface ConnectionError {
  code: ConnectionErrorCode;
  title: string;
  suggestions: string[];
  /** Backend-provided technical reason (never contains credentials). */
  detail?: string;
}

/** SSH host-key metadata for TOFU trust prompts. Non-secret (no credentials). */
export interface HostKeyInfo {
  host: string;
  port: number;
  algorithm: string;
  fingerprint: string;
}

/** Host-key decision a user makes when prompted. */
export type HostKeyDecision =
  | { action: "trustAndConnect"; key: HostKeyInfo }
  | { action: "replaceAndConnect"; key: HostKeyInfo };

/** Outcome of a connection attempt (a device, or a decision the user must make). */
export type ConnectOutcome =
  | { kind: "device"; device: JetsonDevice }
  | { kind: "host_key_unknown"; key: HostKeyInfo }
  | {
      kind: "host_key_changed";
      current: HostKeyInfo;
      previous: HostKeyInfo;
    };

export type RemoteEnvironmentState =
  | "ready"
  | "partial"
  | "broken"
  | "provision_required";

/** Environment facts gathered read-only on the device (snake_case, mirrors script). */
export interface RemoteEnvironmentReport {
  state: RemoteEnvironmentState;
  xrdp_installed: boolean;
  xrdp_version: string;
  xorgxrdp_installed: boolean;
  xorgxrdp_version: string;
  xfce_installed: boolean;
  xrdp_enabled: boolean;
  xrdp_active: boolean;
  xrdp_sesman_active: boolean;
  port_3389_listening: boolean;
  port_3350_listening: boolean;
  xrdp_in_ssl_cert_group: boolean;
  session_configured: boolean;
  xsessionrc_ok: boolean;
  issues: string[];
}

export type ProvisionStage =
  | "checking_environment"
  | "already_ready"
  | "provision_required"
  | "preflight"
  | "uploading"
  | "installing_packages"
  | "configuring_session"
  | "starting_service"
  | "verifying"
  | "complete";

export interface ProvisionEvent {
  stage: ProvisionStage;
  message: string;
  detail?: string;
  progress?: number;
}

/** Result of preparing (check→provision→verify) the remote desktop. */
export type PrepareResult =
  | {
      kind: "ready";
      wasAlreadyReady: boolean;
      environment: RemoteEnvironmentReport;
    }
  | { kind: "hostKeyUnknown"; key: HostKeyInfo }
  | {
      kind: "hostKeyChanged";
      current: HostKeyInfo;
      previous: HostKeyInfo;
    };

/** Process status (mirrors Rust `RdpStatus`). Honest: `running` ≠ authenticated. */
export type RdpStatus =
  | { kind: "notRunning" }
  | { kind: "running" }
  | { kind: "exited"; exitCode: number | null; error: string | null };

/** Outcome of a desktop launch request. */
export type RdpLaunchResult = { kind: "opened" } | { kind: "alreadyRunning" };

/** Connection target for the FreeRDP sidecar. Password is transient, memory-only. */
export interface RdpLaunchInput {
  host: string;
  port?: number;
  username: string;
  password: string;
  /** Stable device identity when launching a remembered v3 device. */
  deviceId?: string | null;
}

/**
 * Thrown by a connection service to signal a recoverable failure.
 * The store maps `code` to user-facing copy — never echoes the password.
 */
export class ConnectionFailure extends Error {
  /** Backend technical detail (ProbeError.message); shown verbatim in the UI. */
  constructor(
    public readonly code: ConnectionErrorCode,
    public readonly detail?: string,
  ) {
    super(`connection failed: ${code}`);
    this.name = "ConnectionFailure";
  }
}

const ERROR_COPY: Record<ConnectionErrorCode, ConnectionError> = {
  ssh_timeout: {
    code: "ssh_timeout",
    title: "Couldn't reach this Jetson",
    suggestions: [
      "The Jetson is powered on",
      "Both devices are on the same network",
      "The IP address is correct",
    ],
  },
  auth_failed: {
    code: "auth_failed",
    title: "Authentication failed",
    suggestions: ["Check your username and password."],
  },
  not_jetson: {
    code: "not_jetson",
    title: "This device doesn't appear to be an NVIDIA Jetson.",
    suggestions: [],
  },
  provision_failed: {
    code: "provision_failed",
    title: "Couldn't prepare the remote desktop.",
    suggestions: [
      "Retry the setup or open diagnostics for more information.",
    ],
  },
  rdp_failed: {
    code: "rdp_failed",
    title: "Couldn't open the Jetson desktop.",
    suggestions: [
      "Retry the connection or open diagnostics for more information.",
    ],
  },
  rdp_client_missing: {
    code: "rdp_client_missing",
    title: "FreeRDP is required",
    suggestions: [
      "FreeRDP is required for this development build.",
      "Install it with: brew install freerdp",
    ],
  },
  rdp_connection_failed: {
    code: "rdp_connection_failed",
    title: "Couldn't reach the Jetson desktop.",
    suggestions: [
      "If macOS shows a Local Network permission prompt, click Allow — the app will retry automatically.",
      "Otherwise check the Jetson is powered on and reachable, then retry.",
    ],
  },
  detection_failed: {
    code: "detection_failed",
    title: "Couldn't read device information.",
    suggestions: ["Check the SSH connection and retry."],
  },
  sudo_required: {
    code: "sudo_required",
    title: "Administrator access is required",
    suggestions: [
      "Administrator access is required to install the remote desktop components.",
    ],
  },
  verification_failed: {
    code: "verification_failed",
    title: "Remote desktop didn't start correctly.",
    suggestions: ["Retry the setup or open diagnostics for more information."],
  },
  saved_password_missing: {
    code: "saved_password_missing",
    title: "Stored password isn't available",
    suggestions: [
      "Go back and enter the device password again.",
      "If the password changed on the Jetson, type the new one and reconnect.",
    ],
  },
  unknown: {
    code: "unknown",
    title: "Something went wrong.",
    suggestions: ["Try again."],
  },
};

/** Map an error code to its display copy. Always safe (no secrets). */
export function describeError(
  code: ConnectionErrorCode,
  detail?: string,
): ConnectionError {
  const base = ERROR_COPY[code] ?? ERROR_COPY.unknown;
  return detail ? { ...base, detail } : base;
}
