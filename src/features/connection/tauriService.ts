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
  const code = (err as { code?: IpcErrorCode } | null)?.code;
  const mapped = (code && CODE_MAP[code]) || "unknown";
  return new ConnectionFailure(mapped);
}

function abortied(): Error & { name: "AbortError" } {
  const e = new Error("aborted") as Error & { name: "AbortError" };
  e.name = "AbortError";
  return e;
}

/**
 * Dev affordance: override the SSH port. macOS won't let a non-root process
 * bind local :22, so an ssh tunnel forwarding the device's :22 has to land on
 * an unprivileged local port (e.g. 2222). Set `VITE_JR_SSH_PORT=2222` when
 * running `cargo tauri dev` against such a tunnel. Defaults to 22 in prod.
 *
 * Dev tunnel mode (KI-004): an unsigned/ad-hoc app binary is silently blocked
 * by macOS Local Network privacy (TCC) from opening LAN sockets, so when the
 * port override is active BOTH planes ride the loopback tunnel: the SSH
 * control plane goes to 127.0.0.1:<VITE_JR_SSH_PORT> and the RDP plane to
 * 127.0.0.1:<VITE_JR_RDP_PORT|3389>, regardless of the LAN IP the user typed.
 * The typed host is kept for device identity/display only.
 */
const SSH_PORT_OVERRIDE = import.meta.env.VITE_JR_SSH_PORT
  ? Number(import.meta.env.VITE_JR_SSH_PORT)
  : null;
const RDP_PORT_OVERRIDE = import.meta.env.VITE_JR_RDP_PORT
  ? Number(import.meta.env.VITE_JR_RDP_PORT)
  : null;

export const TUNNEL_MODE = SSH_PORT_OVERRIDE !== null;
const TUNNEL_HOST = "127.0.0.1";
const SSH_PORT = SSH_PORT_OVERRIDE ?? 22;
const DEFAULT_RDP_PORT = 3389;

/** Dev-bar label for the active tunnel routing (null in prod/direct mode). */
export const devTunnelLabel = TUNNEL_MODE
  ? `${TUNNEL_HOST}:${SSH_PORT} / ${TUNNEL_HOST}:${RDP_PORT_OVERRIDE ?? DEFAULT_RDP_PORT}`
  : null;

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
          host: TUNNEL_MODE ? TUNNEL_HOST : input.host,
          port: SSH_PORT,
          username: input.username,
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
          host: TUNNEL_MODE ? TUNNEL_HOST : input.host,
          port: SSH_PORT,
          username: input.username,
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
    try {
      return await invoke<RdpLaunchResult>("launch_remote_desktop", {
        request: {
          // In dev tunnel mode the RDP plane rides the loopback tunnel too
          // (KI-004); otherwise the typed host is used and Rust defaults the
          // port when `undefined` is dropped by JSON serialization.
          host: TUNNEL_MODE ? TUNNEL_HOST : input.host,
          port: TUNNEL_MODE ? (RDP_PORT_OVERRIDE ?? DEFAULT_RDP_PORT) : input.port,
          username: input.username,
          password: input.password || null,
        },
      });
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