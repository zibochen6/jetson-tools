import {
  ConnectionFailure,
  ConnectionInput,
  ConnectionProgress,
  ConnectOutcome,
  HostKeyDecision,
  HostKeyInfo,
  JetsonDevice,
  PrepareResult,
  ProvisionEvent,
  ProvisionStage,
  RdpLaunchInput,
  RdpLaunchResult,
  RdpStatus,
  RemoteEnvironmentReport,
} from "./types";

// The device-memory seam lives in savedDevice.ts; re-exported here so
// callers that consume the service module can reach it without a second
// import path.
export type { DeviceMemoryGateway } from "./savedDevice";

/**
 * A "scenario" is a dev affordance to exercise mock/error paths. It is NOT
 * part of the real product: `TauriConnectionService` derives outcomes from
 * actual SSH / provisioning / RDP results.
 */
export type MockScenario =
  | "success"
  | "ssh_timeout"
  | "auth_failed"
  | "not_jetson"
  | "provision_failed"
  | "rdp_failed"
  | "launch_failed"
  | "host_key_unknown"
  | "host_key_changed"
  | "provision_needed"
  | "sudo_denied"
  | "verification_failed";

export interface ConnectOptions {
  scenario?: MockScenario;
  signal?: AbortSignal;
  /** A host-key decision the user already made on a prior prompt. */
  hostKeyDecision?: HostKeyDecision;
  onProgress?: (progress: ConnectionProgress) => void;
}

export interface ProvisionOptions {
  scenario?: MockScenario;
  signal?: AbortSignal;
  onEvent?: (event: ProvisionEvent) => void;
}

export interface LaunchOptions {
  scenario?: MockScenario;
  signal?: AbortSignal;
}

/**
 * The abstraction boundary between the UI and "how do we reach a Jetson".
 * Phase 2 ships both `MockConnectionService` and `TauriConnectionService`
 * behind this exact interface — the frontend never knows the difference.
 */
export interface ConnectionService {
  connect(input: ConnectionInput, opts: ConnectOptions): Promise<ConnectOutcome>;
  prepare(input: ConnectionInput, opts: ProvisionOptions): Promise<PrepareResult>;
  launch(input: RdpLaunchInput, opts?: LaunchOptions): Promise<RdpLaunchResult>;
  close(): Promise<void>;
  status(): Promise<RdpStatus>;
}

/** Real-device fixture from the Phase 0 spike (docs/SPIKE_RESULTS.md). */
export const MOCK_DEVICE_FIXTURE: JetsonDevice = {
  host: "",
  hostname: "recomputer",
  model: "reComputer J501 mini",
  architecture: "aarch64",
  ubuntuVersion: "22.04",
  jetpackVersion: "6.2.1",
  l4tVersion: "R36.4",
};

export const MOCK_HOST_KEY: HostKeyInfo = {
  host: "",
  port: 22,
  algorithm: "ssh-ed25519",
  fingerprint: "SHA256:mockmockmockmockmockmockmockmockmockmockmockmock",
};

export type DelayFn = () => number;

const defaultDelay: DelayFn = () => 300 + Math.random() * 500;

interface AbortErrorLike extends Error {
  name: "AbortError";
}

export function isAbortError(err: unknown): boolean {
  return (
    err instanceof Error && (err.name === "AbortError" || err.name === "Aborted")
  );
}

function abortError(): AbortErrorLike {
  const err = new Error("aborted") as AbortErrorLike;
  err.name = "AbortError";
  return err;
}

export { abortError };

/** Resolves after `ms`, rejecting with an AbortError if `signal` fires first. */
export function delay(ms: number, signal?: AbortSignal): Promise<void> {
  return new Promise<void>((resolve, reject) => {
    if (signal?.aborted) {
      reject(abortError());
      return;
    }
    const timer = setTimeout(onDone, ms);
    const onAbort = () => {
      clearTimeout(timer);
      reject(abortError());
    };
    signal?.addEventListener("abort", onAbort, { once: true });
    function onDone() {
      signal?.removeEventListener("abort", onAbort);
      resolve();
    }
  });
}

function mockKeyFor(input: ConnectionInput): HostKeyInfo {
  return { ...MOCK_HOST_KEY, host: input.host, port: 22 };
}

function mockReadyEnvironment(): RemoteEnvironmentReport {
  return {
    state: "ready",
    xrdp_installed: true,
    xrdp_version: "0.9.17",
    xorgxrdp_installed: true,
    xorgxrdp_version: "0.2.17",
    xfce_installed: true,
    xrdp_enabled: true,
    xrdp_active: true,
    port_3389_listening: true,
    session_configured: true,
    xsessionrc_ok: true,
    issues: [],
  };
}

/**
 * Mock implementation for UX validation. Walks the product lifecycle states
 * with small delays and fails at the scenario-mapped step. Delays are injected
 * (`delayFn`) so tests can run instantly.
 */
export class MockConnectionService implements ConnectionService {
  private rdpRunning = false;

  constructor(private delayFn: DelayFn = defaultDelay) {}

  async connect(
    input: ConnectionInput,
    opts: ConnectOptions = {},
  ): Promise<ConnectOutcome> {
    const scenario = opts.scenario ?? "success";
    const { signal, onProgress } = opts;

    // Host-key simulation is surfaced as an immediate outcome (no auth happens
    // yet) so the trust prompt UI can be exercised without a real device.
    if (scenario === "host_key_unknown") {
      await delay(this.delayFn(), signal);
      return { kind: "host_key_unknown", key: mockKeyFor(input) };
    }
    if (scenario === "host_key_changed") {
      await delay(this.delayFn(), signal);
      return {
        kind: "host_key_changed",
        current: mockKeyFor(input),
        previous: {
          ...mockKeyFor(input),
          fingerprint: "SHA256:previouspreviouspreviouspreviousprevious",
        },
      };
    }

    const step = async (
      state: ConnectionProgress["state"],
      message: string,
      detail?: string,
    ) => {
      await delay(this.delayFn(), signal);
      onProgress?.({ state, message, detail });
    };

    await step("connecting_ssh", "Connecting to Jetson", `${input.host}:22`);
    if (scenario === "ssh_timeout") throw new ConnectionFailure("ssh_timeout");

    await step("authenticating", "Authenticating", input.username);
    if (scenario === "auth_failed") throw new ConnectionFailure("auth_failed");

    await step("detecting_device", "Detecting device");
    if (scenario === "not_jetson") throw new ConnectionFailure("not_jetson");

    await step("checking_environment", "Checking remote desktop");
    await step("provisioning", "Preparing desktop");
    if (scenario === "provision_failed")
      throw new ConnectionFailure("provision_failed");

    await step("verifying", "Verifying remote desktop");
    await step("ready", "Jetson ready");

    return {
      kind: "device",
      device: { ...MOCK_DEVICE_FIXTURE, host: input.host },
    };
  }

  async prepare(
    _input: ConnectionInput,
    opts: ProvisionOptions = {},
  ): Promise<PrepareResult> {
    const scenario = opts.scenario ?? "success";
    const { signal, onEvent } = opts;
    const event = async (stage: ProvisionStage) => {
      await delay(this.delayFn(), signal);
      onEvent?.({ stage, message: stage });
    };

    await event("checking_environment");

    if (scenario === "success") {
      await event("already_ready");
      return {
        kind: "ready",
        wasAlreadyReady: true,
        environment: mockReadyEnvironment(),
      };
    }

    await event("provision_required");
    await event("preflight");
    if (scenario === "sudo_denied") {
      throw new ConnectionFailure("sudo_required");
    }
    await event("uploading");
    await event("installing_packages");
    if (scenario === "provision_failed") {
      throw new ConnectionFailure("provision_failed");
    }
    await event("configuring_session");
    await event("starting_service");
    await event("verifying");
    if (scenario === "verification_failed") {
      throw new ConnectionFailure("verification_failed");
    }
    await event("complete");
    return {
      kind: "ready",
      wasAlreadyReady: false,
      environment: mockReadyEnvironment(),
    };
  }

  async launch(
    _input: RdpLaunchInput,
    opts: LaunchOptions = {},
  ): Promise<RdpLaunchResult> {
    const scenario = opts.scenario ?? "success";
    await delay(this.delayFn(), opts.signal);
    if (scenario === "launch_failed") {
      throw new ConnectionFailure("rdp_failed");
    }
    this.rdpRunning = true;
    return { kind: "opened" };
  }

  async close(): Promise<void> {
    this.rdpRunning = false;
  }

  async status(): Promise<RdpStatus> {
    return this.rdpRunning ? { kind: "running" } : { kind: "notRunning" };
  }
}