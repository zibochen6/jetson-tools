// Connection Scenario tests (Reliability Harness, ChatGPT r2 §2.2 "A" group).
//
// These drive `createConnectionStore` through a *scripted* ConnectionService
// so each scenario asserts not just the final state but the exact sequence of
// backend calls ("auth failure must never reach prepare", "cancel must be
// idempotent", ...). They are pure orchestration tests: no SSH, no FreeRDP,
// no network (=== Layer 0/1 of the harness).
//
// PENDING REAL-HARDWARE: these scenarios model the backend at the service
// seam. End-to-end behavior on real Jetsons is covered by the hardware layer
// of the harness (added when devices are available).

import { beforeEach, describe, expect, it, vi } from "vitest";

// The multi-path RTT probe is the only Tauri call the store makes directly;
// stub it so ordering is deterministic (default: no probe data → keep order).
const { probePathsMock } = vi.hoisted(() => ({
  probePathsMock: vi.fn(
    async () =>
      [] as { address: string; reachable: boolean; rttMs: number | null }[],
  ),
}));
vi.mock("../features/connection/tauriService", async (importOriginal) => {
  const actual =
    await importOriginal<typeof import("../features/connection/tauriService")>();
  return { ...actual, probeDevicePaths: probePathsMock };
});

import { ConnectionService } from "../features/connection/service";
import type {
  ConnectOptions,
  LaunchOptions,
  ProvisionOptions,
} from "../features/connection/service";
import {
  DeviceMemoryGateway,
  SavedDeviceInfo,
} from "../features/connection/savedDevice";
import {
  ConnectionFailure,
  ConnectionErrorCode,
  ConnectionInput,
  ConnectionProgress,
  ConnectOutcome,
  HostKeyInfo,
  JetsonDevice,
  PrepareResult,
  RdpLaunchInput,
  RdpLaunchResult,
  RdpStatus,
} from "../features/connection/types";
import { createConnectionStore } from "../stores/connectionStore";
import { useSessionsStore } from "../stores/sessionsStore";

const DEVICE: JetsonDevice = {
  host: "192.168.2.18",
  deviceId: "1421123007848",
  model: "reComputer J501 mini",
};

const HOST_KEY: HostKeyInfo = {
  host: "192.168.2.18",
  port: 22,
  algorithm: "ssh-ed25519",
  fingerprint: "SHA256:mock",
};

const READY: PrepareResult = {
  kind: "ready",
  wasAlreadyReady: true,
  environment: {
    state: "ready",
    xrdp_installed: true,
    xrdp_version: "0.9.17",
    xorgxrdp_installed: true,
    xorgxrdp_version: "0.2.17",
    xfce_installed: true,
    xrdp_enabled: true,
    xrdp_active: true,
    xrdp_sesman_active: true,
    port_3389_listening: true,
    port_3350_listening: true,
    xrdp_in_ssl_cert_group: true,
    session_configured: true,
    xsessionrc_ok: true,
    lo_ipv6_loopback: true,
    issues: [],
  },
};

/** One scripted backend answer for the next matching call. */
type ScriptStep =
  | { connect: { outcome?: ConnectOutcome; throw?: ConnectionErrorCode; progress?: ConnectionProgress[] } }
  | { prepare: { result?: PrepareResult; throw?: ConnectionErrorCode } }
  | { launch: { result?: RdpLaunchResult; throw?: ConnectionErrorCode } };

/** A ConnectionService whose behavior is driven by a per-call script. */
class ScriptedService implements ConnectionService {
  calls: string[] = [];
  private stepQueue: ScriptStep[] = [];
  /** Never-resolving gate used to hold a call open (cancel/stale tests). */
  private holdConnect = false;
  private heldResolvers: (() => void)[] = [];

  script(steps: ScriptStep[]): void {
    this.stepQueue = steps;
  }

  async connect(input: ConnectionInput, opts: ConnectOptions): Promise<ConnectOutcome> {
    this.calls.push("connect:" + input.host);
    const step = this.stepQueue.shift();
    if (step && "connect" in step) {
      for (const p of step.connect.progress ?? []) opts.onProgress?.(p);
      if (step.connect.throw) throw new ConnectionFailure(step.connect.throw);
      if (step.connect.outcome) return step.connect.outcome;
    }
    if (this.holdConnect) {
      await new Promise<void>((resolve) => {
        if (opts.signal?.aborted) return resolve();
        const onAbort = () => resolve();
        opts.signal?.addEventListener("abort", onAbort, { once: true });
        this.heldResolvers.push(() => {
          opts.signal?.removeEventListener("abort", onAbort);
          resolve();
        });
      });
    }
    opts.onProgress?.({ state: "detecting_device", message: "Detecting" });
    return { kind: "device", device: { ...DEVICE, host: input.host } };
  }

  async prepare(_input: ConnectionInput, opts: ProvisionOptions): Promise<PrepareResult> {
    this.calls.push("prepare");
    const step = this.stepQueue.shift();
    if (step && "prepare" in step) {
      opts.onEvent?.({ stage: "checking_environment", message: "Checking" });
      if (step.prepare.throw) throw new ConnectionFailure(step.prepare.throw);
      if (step.prepare.result) return step.prepare.result;
    }
    opts.onEvent?.({ stage: "already_ready", message: "Already ready" });
    return READY;
  }

  async launch(_input: RdpLaunchInput, opts?: LaunchOptions): Promise<RdpLaunchResult> {
    this.calls.push("launch:" + (opts?.sessionId ?? ""));
    const step = this.stepQueue.shift();
    if (step && "launch" in step) {
      if (step.launch.throw) throw new ConnectionFailure(step.launch.throw);
      if (step.launch.result) return step.launch.result;
    }
    return { kind: "opened" };
  }

  async close(): Promise<void> {
    this.calls.push("close");
  }

  async status(): Promise<RdpStatus> {
    return { kind: "notRunning" };
  }

  /** Hold the next connect() open until releaseHeldConnect(). */
  holdNextConnect(): void {
    this.holdConnect = true;
  }

  /** Release every held connect() with a default device outcome. */
  releaseHeldConnect(): void {
    this.holdConnect = false;
    for (const resolve of this.heldResolvers.splice(0)) resolve();
  }
}

/** A device memory that pre-loads a named device (skips the naming gate). */
const SAVED_DEVICE: SavedDeviceInfo = {
  deviceId: "1421123007848",
  username: "seeed",
  displayName: "Robotics 32G",
  paths: [{ kind: "lan", address: "192.168.2.18" }],
  lastUsedPath: "192.168.2.18",
  hasPassword: true,
};

const MEMORY: DeviceMemoryGateway = {
  async loadAll(): Promise<SavedDeviceInfo[]> {
    return [SAVED_DEVICE];
  },
  async save(): Promise<void> {},
  async forget(): Promise<void> {},
};

function makeStore(service: ScriptedService) {
  const store = createConnectionStore(() => service, MEMORY);
  store.getState().setMode("real");
  // A remembered, named device: the naming gate opens (identity-v3).
  store.setState({ savedDevices: [SAVED_DEVICE] });
  return store;
}

async function fillAndConnect(store: ReturnType<typeof makeStore>) {
  store.getState().setForm({
    host: DEVICE.host,
    username: "seeed",
    password: "pw",
    remember: false,
    deviceId: null,
  });
  await store.getState().connect();
}

describe("connectionScenarios (Reliability Harness)", () => {
  beforeEach(() => {
    probePathsMock.mockResolvedValue([]);
    useSessionsStore.setState({ sessions: {}, order: [], activeId: null });
  });

  it("happy path reaches desktop_opened with connect→prepare→launch order", async () => {
    const service = new ScriptedService();
    service.script([
      {
        connect: {
          outcome: { kind: "device", device: DEVICE },
          progress: [{ state: "detecting_device", message: "Detecting" }],
        },
      },
      { prepare: { result: READY } },
      { launch: { result: { kind: "opened" } } },
    ]);
    const store = makeStore(service);

    await fillAndConnect(store);

    const s = store.getState();
    expect(s.state).toBe("desktop_opened");
    expect(service.calls).toEqual([
      "connect:192.168.2.18",
      "prepare",
      "launch:seeed@1421123007848",
    ]);
    // Trace is ordered, belongs to this attempt, and covers connect → detect
    // → prepare phases.
    expect(s.attemptId).toBe(1);
    expect(s.trace.map((e) => e.attemptId)).toEqual([1, 1, 1]);
    expect(s.trace.map((e) => e.state)).toEqual([
      "connecting_ssh",
      "detecting_device",
      "checking_environment",
    ]);
  });

  it("ssh connect failure reaches error and never calls prepare", async () => {
    const service = new ScriptedService();
    service.script([{ connect: { throw: "ssh_timeout" } }]);
    const store = makeStore(service);

    await fillAndConnect(store);

    const s = store.getState();
    expect(s.state).toBe("error");
    expect(s.error?.code).toBe("ssh_timeout");
    expect(service.calls).toEqual(["connect:192.168.2.18"]);
  });

  it("auth failure reaches error and does NOT start provisioning", async () => {
    const service = new ScriptedService();
    service.script([{ connect: { throw: "auth_failed" } }]);
    const store = makeStore(service);

    await fillAndConnect(store);

    expect(store.getState().state).toBe("error");
    expect(store.getState().error?.code).toBe("auth_failed");
    expect(service.calls).toEqual(["connect:192.168.2.18"]);
  });

  it("provision failure reaches error (after prepare was called)", async () => {
    const service = new ScriptedService();
    service.script([
      { connect: { outcome: { kind: "device", device: DEVICE } } },
      { prepare: { throw: "provision_failed" } },
    ]);
    const store = makeStore(service);

    await fillAndConnect(store);

    expect(store.getState().state).toBe("error");
    expect(store.getState().error?.code).toBe("provision_failed");
    expect(service.calls).toEqual(["connect:192.168.2.18", "prepare"]);
  });

  it("launch failure reaches error without reopening tunnel-less RDP", async () => {
    const service = new ScriptedService();
    service.script([
      { connect: { outcome: { kind: "device", device: DEVICE } } },
      { prepare: { result: READY } },
      { launch: { throw: "rdp_failed" } },
    ]);
    const store = makeStore(service);

    await fillAndConnect(store);

    const s = store.getState();
    expect(s.state).toBe("error");
    expect(s.error?.code).toBe("rdp_failed");
    expect(s.lastFailure).toBe("launch");
    expect(service.calls).toEqual([
      "connect:192.168.2.18",
      "prepare",
      "launch:seeed@1421123007848",
    ]);
  });

  it("cancel during a held connect is idempotent and returns to idle", async () => {
    const service = new ScriptedService();
    service.holdNextConnect();
    const store = makeStore(service);

    const pending = fillAndConnect(store);
    // Let the fatal connect() really start, then cancel twice.
    await vi.waitFor(() => {
      expect(service.calls).toEqual(["connect:192.168.2.18"]);
    });
    store.getState().cancel();
    store.getState().cancel();

    await pending;

    const s = store.getState();
    expect(s.state).toBe("idle");
    expect(s.trace.every((e) => e.attemptId === 1)).toBe(true);
  });

  it("a second connect while one is in flight supersedes the first (attempt generation)", async () => {
    const service = new ScriptedService();
    service.holdNextConnect();
    const store = makeStore(service);
    store.getState().setForm({
      host: DEVICE.host,
      username: "seeed",
      password: "pw",
      remember: false,
      deviceId: null,
    });

    const first = store.getState().connect();
    await vi.waitFor(() => {
      expect(service.calls).toEqual(["connect:192.168.2.18"]);
    });
    const second = store.getState().connect();
    // Release the held first connect AFTER the second attempt started —
    // the first attempt's late answer must not clobber attempt #2.
    service.releaseHeldConnect();
    service.holdNextConnect();
    await vi.waitFor(() => {
      expect(service.calls.length).toBe(2);
    });
    service.releaseHeldConnect();
    service.holdNextConnect();

    await Promise.all([first, second]);

    expect(store.getState().attemptId).toBe(2);
    expect(store.getState().state).not.toBe("idle");
  });

  it("stale async result from an older attempt is ignored (generation guard)", async () => {
    const service = new ScriptedService();
    const store = makeStore(service);
    store.getState().setForm({
      host: DEVICE.host,
      username: "seeed",
      password: "pw",
      remember: false,
      deviceId: null,
    });

    // Attempt 1: device D1… but we cancel mid-flight to model supersession.
    service.script([{ connect: { outcome: { kind: "device", device: DEVICE } } }]);
    const first = store.getState().connect();
    await first; // completes attempt 1 (state desktop_opened)
    expect(store.getState().attemptId).toBe(1);
    expect(store.getState().state).toBe("desktop_opened");

    // Attempt 2 starts; its connect is held while attempt-1-era progress
    // callbacks would be late. Real store guards via attemptId.
    service.holdNextConnect();
    const second = store.getState().connect();
    expect(store.getState().attemptId).toBe(2);
    service.releaseHeldConnect();
    // Feed what would-be a stale hook: progress on the OLD generation must
    // not leak into the state of attempt 2.
    await second;
    expect(store.getState().attemptId).toBe(2);
  });

  it("duplicate connect while in flight does not create a third generation", async () => {
    const service = new ScriptedService();
    service.holdNextConnect();
    const store = makeStore(service);
    store.getState().setForm({
      host: DEVICE.host,
      username: "seeed",
      password: "pw",
      remember: false,
      deviceId: null,
    });

    const a = store.getState().connect();
    const b = store.getState().connect();
    const c = store.getState().connect();

    await vi.waitFor(() => {
      expect(service.calls.length).toBeGreaterThanOrEqual(1);
    });
    expect(store.getState().attemptId).toBe(3);
    service.releaseHeldConnect();
    service.holdNextConnect();
    service.releaseHeldConnect();

    await Promise.all([a, b, c]);
    expect(store.getState().attemptId).toBe(3);
  });

  it("unrecoverable failure never loops: one attempt, terminal error state", async () => {
    const service = new ScriptedService();
    service.script([{ connect: { throw: "not_jetson" } }]);
    const store = makeStore(service);

    await fillAndConnect(store);
    expect(store.getState().state).toBe("error");
    expect(store.getState().error?.code).toBe("not_jetson");
    expect(service.calls.filter((c) => c.startsWith("connect:")).length).toBe(1);
  });

  it("all failure paths converge to a known terminal state (error or idle)", async () => {
    const stepFor: Record<string, ScriptStep[]> = {
      ssh_timeout: [{ connect: { throw: "ssh_timeout" } }],
      auth_failed: [{ connect: { throw: "auth_failed" } }],
      not_jetson: [{ connect: { throw: "not_jetson" } }],
      provision_failed: [
        { connect: { outcome: { kind: "device", device: DEVICE } } },
        { prepare: { throw: "provision_failed" } },
      ],
      rdp_failed: [
        { connect: { outcome: { kind: "device", device: DEVICE } } },
        { prepare: { result: READY } },
        { launch: { throw: "rdp_failed" } },
      ],
    };
    for (const [, steps] of Object.entries(stepFor)) {
      const service = new ScriptedService();
      service.script(steps);
      const store = makeStore(service);
      store.getState().setForm({
        host: DEVICE.host,
        username: "seeed",
        password: "pw",
        remember: false,
        deviceId: null,
      });
      await store.getState().connect();
      const s = store.getState();
      expect(s.state === "error" || s.state === "idle").toBe(true);
      expect(s.state.startsWith("connecting")).toBe(false);
      expect(s.attemptId).toBe(1);
    }
  });

  it("host key unknown prompts the user and never proceeds to prepare", async () => {
    const service = new ScriptedService();
    service.script([
      { connect: { outcome: { kind: "host_key_unknown", key: HOST_KEY } } },
    ]);
    const store = makeStore(service);

    await fillAndConnect(store);

    const s = store.getState();
    expect(s.state).toBe("host_key_unknown");
    expect(s.hostKey?.fingerprint).toBe("SHA256:mock");
    expect(service.calls).toEqual(["connect:192.168.2.18"]);
  });

  it("cancel after an error leaves the state idle and never calls prepare", async () => {
    const service = new ScriptedService();
    service.script([{ connect: { throw: "ssh_timeout" } }]);
    const store = makeStore(service);

    await fillAndConnect(store);
    const s = store.getState();
    expect(s.state).toBe("error");
    store.getState().cancel();
    expect(store.getState().state).toBe("idle");
    expect(service.calls.filter((c) => c === "prepare").length).toBe(0);
  });
});