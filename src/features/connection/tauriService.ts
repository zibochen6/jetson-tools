import { Channel, invoke } from "@tauri-apps/api/core";
import {
  ConnectOptions,
  ConnectionService,
  LaunchOptions,
  ProvisionOptions,
} from "./service";
import {
  ConnectionErrorCode,
  ConnectionFailure,
  ConnectionInput,
  ConnectOutcome,
  HostKeyDecision,
  HostKeyInfo,
  JetsonDevice,
  PrepareResult,
  ProvisionEvent,
  RdpLaunchInput,
  RdpLaunchResult,
  RdpStatus,
  RemoteEnvironmentReport,
} from "./types";

/** Rust `ProbeErrorCode` (SCREAMING_SNAKE_CASE) → product error code. */
type ProbeErrorCode =
  | "SSH_TIMEOUT"
  | "CONNECTION_REFUSED"
  | "AUTHENTICATION_FAILED"
  | "SSH_PROTOCOL_ERROR"
  | "REMOTE_COMMAND_FAILED"
  | "DETECTION_PARSE_FAILED"
  | "NOT_A_JETSON"
  | "SUDO_AUTHENTICATION_FAILED"
  | "SUDO_PERMISSION_DENIED"
  | "PROVISION_FAILED"
  | "PROVISION_TIMEOUT"
  | "VERIFICATION_FAILED"
  | "SAVED_PASSWORD_MISSING"
  | "CANCELLED"
  | "UNKNOWN";

const CODE_MAP: Record<IpcErrorCode, ConnectionErrorCode> = {
  SSH_TIMEOUT: "ssh_timeout",
  CONNECTION_REFUSED: "ssh_timeout",
  AUTHENTICATION_FAILED: "auth_failed",
  SSH_PROTOCOL_ERROR: "detection_failed",
  REMOTE_COMMAND_FAILED: "detection_failed",
  DETECTION_PARSE_FAILED: "detection_failed",
  NOT_A_JETSON: "not_jetson",
  SUDO_AUTHENTICATION_FAILED: "sudo_required",
  SUDO_PERMISSION_DENIED: "sudo_required",
  PROVISION_FAILED: "provision_failed",
  PROVISION_TIMEOUT: "provision_failed",
  VERIFICATION_FAILED: "verification_failed",
  SAVED_PASSWORD_MISSING: "saved_password_missing",
  CANCELLED: "unknown",
  UNKNOWN: "unknown",
  RDP_CLIENT_NOT_FOUND: "rdp_client_missing",
  RDP_CLIENT_VERSION_UNSUPPORTED: "rdp_failed",
  RDP_LAUNCH_FAILED: "rdp_failed",
  RDP_AUTHENTICATION_FAILED: "rdp_failed",
  RDP_CERTIFICATE_CHANGED: "rdp_failed",
  RDP_CONNECTION_FAILED: "rdp_failed",
  RDP_PROCESS_CRASHED: "rdp_failed",
  RDP_PASSWORD_MISSING: "saved_password_missing",
  RDP_ALREADY_RUNNING: "rdp_failed",
  RDP_UNKNOWN: "rdp_failed",
};

type IpcErrorCode = ProbeErrorCode | RdpErrorCode;

type RdpErrorCode =
  | "RDP_CLIENT_NOT_FOUND"
  | "RDP_CLIENT_VERSION_UNSUPPORTED"
  | "RDP_LAUNCH_FAILED"
  | "RDP_AUTHENTICATION_FAILED"
  | "RDP_CERTIFICATE_CHANGED"
  | "RDP_CONNECTION_FAILED"
  | "RDP_PROCESS_CRASHED"
  | "RDP_PASSWORD_MISSING"
  | "RDP_ALREADY_RUNNING"
  | "RDP_UNKNOWN";

type RustProbeResult =
  | { kind: "device"; device: JetsonDevice }
  | { kind: "hostKeyUnknown"; key: HostKeyInfo }
  | { kind: "hostKeyChanged"; current: HostKeyInfo; previous: HostKeyInfo };

type RustPrepareResult =
  | {
      kind: "ready";
      wasAlreadyReady: boolean;
      environment: RemoteEnvironmentReport;
    }
  | { kind: "hostKeyUnknown"; key: HostKeyInfo }
  | { kind: "hostKeyChanged"; current: HostKeyInfo; previous: HostKeyInfo };

function mapError(err: unknown): ConnectionFailure {
  const e = (err as { code?: IpcErrorCode; message?: string } | null) ?? {};
  const mapped = (e.code && CODE_MAP[e.code]) || "unknown";
  // Surface the backend's technical reason (never the password) so the error
  // screen can tell "unreachable" from "auth" from "sudo" at a glance.
  return new ConnectionFailure(mapped, e.message);
}

function abortied(): Error & { name: "AbortError" } {
  const e = new Error("aborted") as Error & { name: "AbortError" };
  e.name = "AbortError";
  return e;
}

/**
 * The frontend always sends the host the user typed; the Rust backend
 * transparently routes BOTH planes through an in-app loopback ssh tunnel
 * (system `/usr/bin/ssh`, KI-021) so unsigned builds are not blocked by
 * macOS Local Network privacy (KI-004) and no manual `ssh -L` is needed.
 */
const DEFAULT_SSH_PORT = 22;

/**
 * Real SSH control plane. `connect` probes/detects; `prepare` checks the
 * environment, provisions it if needed, and verifies it — streaming progress
 * via a Tauri IPC channel. Never echoes passwords; errors are typed.
 */
export class TauriConnectionService implements ConnectionService {
  async connect(
    input: ConnectionInput,
    opts: ConnectOptions = {},
  ): Promise<ConnectOutcome> {
    const { signal, hostKeyDecision } = opts;
    if (signal?.aborted) throw abortied();

    try {
      const result = await invoke<RustProbeResult>("probe_device", {
        input: {
          host: input.host,
          port: DEFAULT_SSH_PORT,
          username: input.username,
          // Stable identity when connecting a remembered v3 device; the
          // backend resolves the stored password by `user@deviceId`.
          deviceId: input.deviceId ?? null,
          // Empty = the backend resolves the remembered password itself;
          // the stored secret never comes back to the frontend (V0.3).
          password: input.password || null,
        },
        hostKeyDecision: hostKeyDecision ?? null,
      });

      switch (result.kind) {
        case "device":
          return { kind: "device", device: result.device };
        case "hostKeyUnknown":
          return { kind: "host_key_unknown", key: result.key };
        case "hostKeyChanged":
          return {
            kind: "host_key_changed",
            current: result.current,
            previous: result.previous,
          };
      }
    } catch (err) {
      if (signal?.aborted) throw abortied();
      throw mapError(err);
    }
  }

  async prepare(
    input: ConnectionInput,
    opts: ProvisionOptions = {},
  ): Promise<PrepareResult> {
    const { signal, onEvent } = opts;
    if (signal?.aborted) throw abortied();

    const channel = new Channel<ProvisionEvent>();
    channel.onmessage = (event) => {
      if (!signal?.aborted) onEvent?.(event);
    };

    try {
      const result = await invoke<RustPrepareResult>("prepare_remote_desktop", {
        input: {
          host: input.host,
          port: DEFAULT_SSH_PORT,
          username: input.username,
          deviceId: input.deviceId ?? null,
          password: input.password || null,
        },
        hostKeyDecision: null as HostKeyDecision | null,
        onEvent: channel,
      });

      switch (result.kind) {
        case "ready":
          return {
            kind: "ready",
            wasAlreadyReady: result.wasAlreadyReady,
            environment: result.environment,
          };
        case "hostKeyUnknown":
          return { kind: "hostKeyUnknown", key: result.key };
        case "hostKeyChanged":
          return {
            kind: "hostKeyChanged",
            current: result.current,
            previous: result.previous,
          };
      }
    } catch (err) {
      if (signal?.aborted) throw abortied();
      throw mapError(err);
    }
  }

  async launch(
    input: RdpLaunchInput,
    opts: LaunchOptions = {},
  ): Promise<RdpLaunchResult> {
    if (opts.signal?.aborted) throw abortied();
    const request = {
      // The backend routes the RDP plane through the in-app loopback
      // tunnel (KI-021); the typed host is kept for identity only.
      host: input.host,
      username: input.username,
      // Stable identity: drives the tunnel device key + password lookup.
      deviceId: input.deviceId ?? null,
      password: input.password || null,
    };
    try {
      // Multi-device (V0.4): a sessionId routes to the keyed session manager;
      // without one the legacy single-desktop command is used unchanged.
      if (opts.sessionId) {
        return await invoke<RdpLaunchResult>("launch_session", {
          sessionId: opts.sessionId,
          request,
          focusOnLaunch: opts.focusOnLaunch ?? true,
        });
      }
      return await invoke<RdpLaunchResult>("launch_remote_desktop", { request });
    } catch (err) {
      if (opts.signal?.aborted) throw abortied();
      throw mapError(err);
    }
  }

  async close(): Promise<void> {
    try {
      await invoke("close_remote_desktop");
    } catch (err) {
      throw mapError(err);
    }
  }

  async status(): Promise<RdpStatus> {
    return invoke<RdpStatus>("rdp_status");
  }
}

/** Raw TCP reachability probe result (mirrors Rust `TcpProbe`, camelCase). */
export interface NetworkProbe {
  host: string;
  port: number;
  connected: boolean;
  errorKind: string | null;
  osErrno: number | null;
  detail: string;
}

/**
 * Dev-only: raw TCP reachability probe run inside the app process (same
 * identity as the app). Returns the OS errno verbatim — NOT a product error.
 */
export async function networkProbe(
  host: string,
  port: number,
): Promise<NetworkProbe> {
  return invoke<NetworkProbe>("network_probe", { host, port });
}
/**
 * One row of `probe_device_paths`: raw TCP RTT to `address:22` (identity-v3
 * multi-path routing). Advisory only — never a product error.
 */
export interface PathProbeEntry {
  address: string;
  reachable: boolean;
  rttMs: number | null;
}

/**
 * Probe every candidate address of a device in parallel (TCP `:22`). The
 * caller orders candidates by RTT and tries them in sequence.
 */
export async function probeDevicePaths(
  addresses: string[],
): Promise<PathProbeEntry[]> {
  const unique = addresses.filter((a, i) => a.trim() && addresses.indexOf(a) === i);
  if (unique.length === 0) return [];
  try {
    return await invoke<PathProbeEntry[]>("probe_device_paths", {
      addresses: unique,
    });
  } catch {
    // Outside Tauri / transient IPC failure — the caller keeps the original
    // candidate order instead.
    return [];
  }
}

/* ------------------------------------------------------------------ */
/* Multi-device sessions (V0.4)                                       */
/* ------------------------------------------------------------------ */

/** One row of `all_session_statuses` (camelCase mirrors Rust serde). */
export interface SessionStatusEntry {
  sessionId: string;
  status: RdpStatus;
}

/**
 * Session-scoped desktop control for the multi-device tab bar. Every call is
 * best-effort outside Tauri (plain browser dev degrades to no-ops instead of
 * throwing, same policy as `lib/ipc.ts`).
 */
export class TauriSessionService {
  /** Re-open/relaunch one device's desktop under its session key. */
  async launch(
    sessionId: string,
    input: RdpLaunchInput,
    options: { focusOnLaunch: boolean },
  ): Promise<RdpLaunchResult> {
    try {
      return await invoke<RdpLaunchResult>("launch_session", {
        sessionId,
        focusOnLaunch: options.focusOnLaunch,
        request: {
          host: input.host,
          username: input.username,
          deviceId: input.deviceId ?? null,
          password: input.password || null,
        },
      });
    } catch (err) {
      throw mapError(err);
    }
  }

  /** Quick-switch the on-screen desktop; `null` shows the webview home. */
  async focus(sessionId: string | null): Promise<void> {
    try {
      await invoke("focus_session", { sessionId });
    } catch {
      // outside Tauri / transient — UI state stays authoritative
    }
  }

  /** Close one device's session (tab ×). */
  async close(sessionId: string): Promise<void> {
    try {
      await invoke("close_session", { sessionId });
    } catch {
      // best-effort: the tab is removed either way
    }
  }

  /** Snapshot of every backend session (tab-bar polling). */
  async allStatuses(): Promise<SessionStatusEntry[]> {
    return invoke<SessionStatusEntry[]>("all_session_statuses");
  }
}
