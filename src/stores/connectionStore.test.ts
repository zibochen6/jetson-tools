import { beforeEach, describe, expect, it, vi } from "vitest";

// The multi-path RTT probe is the only Tauri call the store makes directly;
// stub it so ordering is deterministic (default: no probe data → keep order).
const { probePathsMock } = vi.hoisted(() => ({
  // Default: "no probe data" (same as the real gateway outside Tauri) so
  // candidate order is preserved unless a test overrides it.
  probePathsMock: vi.fn(async () => [] as { address: string; reachable: boolean; rttMs: number | null }[]),
}));
vi.mock("../features/connection/tauriService", async (importOriginal) => {
  const actual =
    await importOriginal<typeof import("../features/connection/tauriService")>();
  return { ...actual, probeDevicePaths: probePathsMock };
});

import { ConnectionService } from "../features/connection/service";
import { DeviceMemoryGateway, SavedDeviceInfo } from "../features/connection/savedDevice";
import {
  ConnectionFailure,
  ConnectOutcome,
  JetsonDevice,
  PrepareResult,
  RdpLaunchResult,
  RdpStatus,
  RemoteEnvironmentReport,
} from "../features/connection/types";
import { ConnectionForm } from "../features/connection/validation";
import { createConnectionStore, useConnectionStore } from "./connectionStore";
import { useSessionsStore } from "./sessionsStore";

const DEVICE: JetsonDevice = { host: "", model: "reComputer J501 mini" };

/** Identity-v3 device: machine-id + the paths it currently reports. */
const DEVICE_V3: JetsonDevice = {
  host: "192.168.2.18",
  deviceId: "5dbfb12400000000",
  paths: [
    { kind: "lan", address: "192.168.2.18" },
    { kind: "tailscale", address: "100.114.170.49" },
  ],
  model: "reComputer J501",
};

const ENV: RemoteEnvironmentReport = {
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
  issues: [],
};

type Behavior =
  | "success"
  | "auth_failed"
  | "host_key_unknown"
  | "sudo_denied"
  | "provision_failed"
  | "verification_failed"
  | "launch_failed";

function controllableService(device: JetsonDevice = DEVICE) {
  let behavior: Behavior = "success";
  let rdpRunning = false;
  let rdpExited = false;
  const launches: { sessionId?: string; host: string }[] = [];
  const connectHosts: string[] = [];

  const service: ConnectionService = {
    async connect(input, opts): Promise<ConnectOutcome> {
      connectHosts.push(input.host);
      if (behavior === "auth_failed") throw new ConnectionFailure("auth_failed");
      if (behavior === "host_key_unknown") {
        return {
          kind: "host_key_unknown",
          key: {
            host: input.host,
            port: 22,
            algorithm: "ssh-ed25519",
            fingerprint: "SHA256:mock",
          },
        };
      }
      opts.onProgress?.({ state: "connecting_ssh", message: "Connecting" });
      opts.onProgress?.({ state: "detecting_device", message: "Detecting" });
      return { kind: "device", device: { ...device, host: input.host } };
    },
    async prepare(_input, opts): Promise<PrepareResult> {
      opts.onEvent?.({ stage: "checking_environment", message: "Checking" });
      if (behavior === "sudo_denied") {
        opts.onEvent?.({ stage: "provision_required", message: "Setup needed" });
        opts.onEvent?.({ stage: "preflight", message: "Checking admin" });
        throw new ConnectionFailure("sudo_required");
      }
      if (behavior === "provision_failed") {
        opts.onEvent?.({ stage: "provision_required", message: "Setup needed" });
        opts.onEvent?.({
          stage: "installing_packages",
          message: "Installing",
        });
        throw new ConnectionFailure("provision_failed");
      }
      if (behavior === "verification_failed") {
        opts.onEvent?.({ stage: "provision_required", message: "Setup needed" });
        opts.onEvent?.({ stage: "verifying", message: "Verifying" });
        throw new ConnectionFailure("verification_failed");
      }
      opts.onEvent?.({ stage: "already_ready", message: "Already ready" });
      return { kind: "ready", wasAlreadyReady: true, environment: ENV };
    },
    async launch(input, opts): Promise<RdpLaunchResult> {
      launches.push({ sessionId: opts?.sessionId, host: input.host });
      if (behavior === "launch_failed") throw new ConnectionFailure("rdp_failed");
      rdpRunning = true;
      rdpExited = false;
      return { kind: "opened" };
    },
    async close(): Promise<void> {
      rdpRunning = false;
    },
    async status(): Promise<RdpStatus> {
      if (rdpExited) return { kind: "exited", exitCode: 0, error: null };
      return rdpRunning ? { kind: "running" } : { kind: "notRunning" };
    },
  };
  return {
    service,
    setBehavior: (b: Behavior) => {
      behavior = b;
    },
    /** Simulate the desktop process exiting on its own (window closed). */
    exitDesktop: () => {
      rdpExited = true;
      rdpRunning = false;
    },
    launches,
    connectHosts,
  };
}

function deferredService() {
  let resolveFn: ((v: ConnectOutcome) => void) | null = null;
  const service: ConnectionService = {
    connect: () =>
      new Promise<ConnectOutcome>((resolve) => {
        resolveFn = resolve;
      }),
    prepare: async () => {
      throw new ConnectionFailure("unknown");
    },
    launch: async () => ({ kind: "opened" as const }),
    close: async () => {},
    status: async () => ({ kind: "notRunning" as const }),
  };
  return {
    service,
    resolve: (v: ConnectOutcome) => resolveFn?.(v),
  };
}

function filledForm(): ConnectionForm {
  return {
    host: "192.168.100.164",
    username: "seeed",
    password: "secret",
    remember: false,
    deviceId: null,
  };
}

/** In-memory device-memory gateway; records calls for assertions. */
function fakeMemory(initial: SavedDeviceInfo[] = []) {
  const calls: string[] = [];
  let memory = [...initial];
  const gateway: DeviceMemoryGateway = {
    async loadAll() {
      calls.push("loadAll");
      return memory;
    },
    async save(input) {
      calls.push(
        `save:${input.deviceId ?? input.entryHost}:${input.username}:${input.password}:${input.displayName ?? ""}`,
      );
      const entry: SavedDeviceInfo = {
        deviceId: input.deviceId,
        username: input.username,
        displayName: input.displayName,
        paths: input.paths,
        lastUsedPath: input.entryHost,
        hasPassword: true,
      };
      memory = [
        entry,
        ...memory.filter(
          (device) =>
            !(
              device.username === entry.username &&
              ((entry.deviceId && device.deviceId === entry.deviceId) ||
                (!entry.deviceId &&
                  device.lastUsedPath === entry.lastUsedPath))
            ),
        ),
      ];
    },
    async forget(input) {
      calls.push(`forget:${input.deviceId ?? input.host}:${input.username}`);
      memory = memory.filter(
        (device) =>
          !(
            device.username === input.username &&
            ((input.deviceId && device.deviceId === input.deviceId) ||
              (!input.deviceId && device.lastUsedPath === input.host))
          ),
      );
    },
  };
  return {
    gateway,
    calls,
    setMemory: (m: SavedDeviceInfo[]) => {
      memory = m;
    },
  };
}

/** A remembered v3 device: named, two paths, stored password. */
function savedDeviceFixture(): SavedDeviceInfo {
  return {
    deviceId: "5dbfb12400000000",
    username: "seeed",
    displayName: "robotics",
    paths: [
      { kind: "lan", address: "192.168.2.18" },
      { kind: "tailscale", address: "100.114.170.49" },
    ],
    lastUsedPath: "192.168.2.18",
    hasPassword: true,
  };
}

/** A legacy v2-shaped entry (no machine-id, no name). */
function legacySavedFixture(): SavedDeviceInfo {
  return {
    deviceId: null,
    username: "seeed",
    displayName: null,
    paths: [{ kind: "lan", address: "192.168.100.164" }],
    lastUsedPath: "192.168.100.164",
    hasPassword: true,
  };
}

async function flush() {
  for (let i = 0; i < 10; i++) await Promise.resolve();
}

describe("connectionStore state machine", () => {
  it("probe + prepare + auto-launch ends desktop_opened", async () => {
    const store = createConnectionStore(() => controllableService().service);
    store.getState().setForm(filledForm());
    await store.getState().connect();

    const s = store.getState();
    expect(s.state).toBe("desktop_opened");
    expect(s.device?.host).toBe("192.168.100.164");
    expect(s.environment?.state).toBe("ready");
    expect(s.error).toBeNull();
  });

  it("errors on auth failure, then retry recovers to desktop", async () => {
    const ctl = controllableService();
    ctl.setBehavior("auth_failed");
    const store = createConnectionStore(() => ctl.service);
    store.getState().setForm(filledForm());

    await store.getState().connect();
    expect(store.getState().state).toBe("error");
    expect(store.getState().error?.code).toBe("auth_failed");

    ctl.setBehavior("success");
    store.getState().retry();
    await flush();
    expect(store.getState().state).toBe("desktop_opened");
  });

  it("prompts for host key, then trust proceeds to desktop", async () => {
    const ctl = controllableService();
    ctl.setBehavior("host_key_unknown");
    const store = createConnectionStore(() => ctl.service);
    store.getState().setForm(filledForm());

    await store.getState().connect();
    expect(store.getState().state).toBe("host_key_unknown");

    ctl.setBehavior("success");
    store.getState().trustKey();
    await flush();
    expect(store.getState().state).toBe("desktop_opened");
  });

  it("maps sudo denial to sudo_required error", async () => {
    const ctl = controllableService();
    ctl.setBehavior("sudo_denied");
    const store = createConnectionStore(() => ctl.service);
    store.getState().setForm(filledForm());

    await store.getState().connect();
    expect(store.getState().state).toBe("error");
    expect(store.getState().error?.code).toBe("sudo_required");
  });

  it("maps provision failure to provision_failed error", async () => {
    const ctl = controllableService();
    ctl.setBehavior("provision_failed");
    const store = createConnectionStore(() => ctl.service);
    store.getState().setForm(filledForm());

    await store.getState().connect();
    expect(store.getState().error?.code).toBe("provision_failed");
  });

  it("launch failure keeps device and retry relaunches (not reconnect)", async () => {
    const ctl = controllableService();
    ctl.setBehavior("launch_failed");
    const store = createConnectionStore(() => ctl.service);
    store.getState().setForm(filledForm());

    await store.getState().connect();
    expect(store.getState().state).toBe("error");
    expect(store.getState().error?.code).toBe("rdp_failed");
    // provision succeeded, so the device (and ready env) is preserved
    expect(store.getState().device?.host).toBe("192.168.100.164");

    ctl.setBehavior("success");
    store.getState().retry();
    await flush();
    expect(store.getState().state).toBe("desktop_opened");
  });

  it("close desktop returns to ready (session persists, device kept)", async () => {
    const store = createConnectionStore(() => controllableService().service);
    store.getState().setForm(filledForm());
    await store.getState().connect();
    expect(store.getState().state).toBe("desktop_opened");

    await store.getState().closeDesktop();
    expect(store.getState().state).toBe("ready");
    expect(store.getState().device?.host).toBe("192.168.100.164");
  });

  it("detects process exit via refreshRdpStatus and returns to ready", async () => {
    const ctl = controllableService();
    const store = createConnectionStore(() => ctl.service);
    store.getState().setForm(filledForm());
    await store.getState().connect();
    expect(store.getState().state).toBe("desktop_opened");

    ctl.exitDesktop();
    await store.getState().refreshRdpStatus();
    expect(store.getState().state).toBe("ready");
    expect(store.getState().device?.host).toBe("192.168.100.164");
  });

  it("ignores stale exit once back at ready", async () => {
    const ctl = controllableService();
    const store = createConnectionStore(() => ctl.service);
    store.getState().setForm(filledForm());
    await store.getState().connect();

    await store.getState().closeDesktop();
    expect(store.getState().state).toBe("ready");

    // a late exit signal must not rebind to idle/error
    ctl.exitDesktop();
    await store.getState().refreshRdpStatus();
    expect(store.getState().state).toBe("ready");
  });

  it("disconnect clears device and returns to idle", async () => {
    const store = createConnectionStore(() => controllableService().service);
    store.getState().setForm(filledForm());
    await store.getState().connect();
    expect(store.getState().state).toBe("desktop_opened");

    await store.getState().disconnect();
    expect(store.getState().state).toBe("idle");
    expect(store.getState().device).toBeNull();
    expect(store.getState().environment).toBeNull();
  });

  it("cancel (before bootstrap) prevents a late result from rebinding", async () => {
    const d = deferredService();
    const store = createConnectionStore(() => d.service);
    store.getState().setForm(filledForm());

    const p = store.getState().connect();
    store.getState().cancel();
    d.resolve({ kind: "device", device: { ...DEVICE, host: "192.168.100.164" } });
    await p;

    expect(store.getState().state).toBe("idle");
  });

  it("does not apply zustand persist middleware (password never persisted)", () => {
    expect(
      (useConnectionStore as unknown as { persist?: unknown }).persist,
    ).toBeUndefined();
  });
});

describe("remembered device memory", () => {
  it("connect with remember + typed password saves the device", async () => {
    const mem = fakeMemory();
    const store = createConnectionStore(
      () => controllableService().service,
      mem.gateway,
    );
    store.getState().setForm({ ...filledForm(), remember: true });

    await store.getState().connect();
    expect(store.getState().state).toBe("desktop_opened");
    // The mock device has no machine-id: identity falls back to the entry host.
    expect(mem.calls).toContain("save:192.168.100.164:seeed:secret:");
    expect(store.getState().savedDevices).toEqual([
      {
        deviceId: null,
        username: "seeed",
        displayName: null,
        paths: [],
        lastUsedPath: "192.168.100.164",
        hasPassword: true,
      },
    ]);
  });

  it("blank password with remembered device refreshes paths without rewriting the secret", async () => {
    const mem = fakeMemory([legacySavedFixture()]);
    const store = createConnectionStore(
      () => controllableService().service,
      mem.gateway,
    );
    store.setState({ savedDevices: [legacySavedFixture()] });
    store.getState().setForm({
      host: "192.168.100.164",
      username: "seeed",
      password: "",
      remember: true,
      deviceId: null,
    });

    await store.getState().connect();
    expect(store.getState().state).toBe("desktop_opened");
    // Identity-v3: the path list is refreshed (empty password keeps the secret).
    expect(mem.calls).toContain("save:192.168.100.164:seeed::");
    expect(store.getState().savedDevices.length).toBe(1);
  });

  it("unchecking remember on the remembered device forgets it", async () => {
    const mem = fakeMemory([legacySavedFixture()]);
    const store = createConnectionStore(
      () => controllableService().service,
      mem.gateway,
    );
    store.getState().setForm({ ...filledForm(), remember: false });
    // seed the store state so the match check fires
    store.setState({ savedDevices: [legacySavedFixture()] });

    await store.getState().connect();
    expect(store.getState().state).toBe("desktop_opened");
    expect(mem.calls).toContain("forget:192.168.100.164:seeed");
    expect(store.getState().savedDevices).toEqual([]);
  });

  it("unchecking remember on a DIFFERENT device keeps the memory", async () => {
    const mem = fakeMemory([savedDeviceFixture()]);
    const store = createConnectionStore(
      () => controllableService().service,
      mem.gateway,
    );
    store.getState().setForm({
      host: "192.168.1.99",
      username: "other",
      password: "pw",
      remember: false,
      deviceId: null,
    });
    store.setState({ savedDevices: [savedDeviceFixture()] });

    await store.getState().connect();
    expect(store.getState().state).toBe("desktop_opened");
    expect(mem.calls).not.toContain("forget:192.168.1.99:other");
    expect(store.getState().savedDevices).toEqual([savedDeviceFixture()]);
  });

  it("memory save failure never breaks the connection flow", async () => {
    const mem = fakeMemory();
    const failing: DeviceMemoryGateway = {
      loadAll: mem.gateway.loadAll,
      save: async () => {
        throw new Error("disk full");
      },
      forget: async () => {
        throw new Error("disk full");
      },
    };
    const store = createConnectionStore(
      () => controllableService().service,
      failing,
    );
    store.getState().setForm({ ...filledForm(), remember: true });

    await store.getState().connect();
    expect(store.getState().state).toBe("desktop_opened");
  });

  it("initRemembered with stored password prefills and auto-connects", async () => {
    const mem = fakeMemory([savedDeviceFixture()]);
    const store = createConnectionStore(
      () => controllableService().service,
      mem.gateway,
    );

    await store.getState().initRemembered();
    await flush(); // auto-connect resolves asynchronously
    const s = store.getState();
    expect(s.form).toMatchObject({
      host: "192.168.2.18",
      username: "seeed",
      password: "",
      remember: true,
      deviceId: "5dbfb12400000000",
    });
    expect(s.state).toBe("desktop_opened");
  });

  it("initRemembered without a password prefills but stays idle", async () => {
    const mem = fakeMemory([{
      ...legacySavedFixture(),
      hasPassword: false,
    }]);
    const store = createConnectionStore(
      () => controllableService().service,
      mem.gateway,
    );

    await store.getState().initRemembered();
    const s = store.getState();
    expect(s.form.host).toBe("192.168.100.164");
    expect(s.form.username).toBe("seeed");
    expect(s.state).toBe("idle");
  });

  it("initRemembered is a once-per-run guard (StrictMode safe)", async () => {
    const mem = fakeMemory([savedDeviceFixture()]);
    let connects = 0;
    const counting = controllableService();
    const original = counting.service.connect.bind(counting.service);
    counting.service.connect = async (input, opts) => {
      connects += 1;
      return original(input, opts);
    };
    const store = createConnectionStore(() => counting.service, mem.gateway);

    await Promise.all([
      store.getState().initRemembered(),
      store.getState().initRemembered(),
    ]);
    expect(connects).toBe(1);
  });

  it("forgetDevice clears memory by device identity", async () => {
    const mem = fakeMemory([savedDeviceFixture()]);
    const store = createConnectionStore(
      () => controllableService().service,
      mem.gateway,
    );
    await store.getState().initRemembered();
    // Let the auto-connect finish: a blank-password reconnect now refreshes
    // the memory entry, which would otherwise race the forget below.
    await flush();

    await store.getState().forgetDevice(savedDeviceFixture());
    expect(mem.calls).toContain("forget:5dbfb12400000000:seeed");
    expect(store.getState().savedDevices).toEqual([]);
  });

  it("forgetDevice falls back to the host for legacy entries", async () => {
    const mem = fakeMemory([legacySavedFixture()]);
    const store = createConnectionStore(
      () => controllableService().service,
      mem.gateway,
    );
    await store.getState().forgetDevice(legacySavedFixture());
    expect(mem.calls).toContain("forget:192.168.100.164:seeed");
    expect(store.getState().savedDevices).toEqual([]);
  });
});

describe("identity-v3: machine-id, naming, multi-path", () => {
  beforeEach(() => {
    probePathsMock.mockResolvedValue([]);
    useSessionsStore.setState({ sessions: {}, order: [], activeId: null });
  });

  it("a new machine-id must be named before the desktop opens", async () => {
    const ctl = controllableService(DEVICE_V3);
    const mem = fakeMemory();
    const store = createConnectionStore(() => ctl.service, mem.gateway);
    store.getState().setForm({
      host: "192.168.2.18",
      username: "seeed",
      password: "pw",
      remember: true,
      deviceId: null,
    });

    await store.getState().connect();
    expect(store.getState().state).toBe("naming_device");
    expect(ctl.launches).toHaveLength(0);

    // Blank names are rejected — the gate cannot be skipped.
    expect(await store.getState().confirmDeviceName("   ")).toBe(false);
    expect(store.getState().state).toBe("naming_device");

    expect(await store.getState().confirmDeviceName("robotics")).toBe(true);
    expect(store.getState().state).toBe("desktop_opened");
    expect(
      mem.calls.some((c) =>
        c.startsWith("save:5dbfb12400000000:seeed:pw:robotics"),
      ),
    ).toBe(true);
    // The session key uses the deviceId, not the IP.
    expect(ctl.launches[0].sessionId).toBe("seeed@5dbfb12400000000");
  });

  it("a remembered, named device skips the naming gate", async () => {
    const ctl = controllableService(DEVICE_V3);
    const mem = fakeMemory([savedDeviceFixture()]);
    const store = createConnectionStore(() => ctl.service, mem.gateway);
    store.setState({ savedDevices: [savedDeviceFixture()] });
    store.getState().setForm({
      host: "192.168.2.18",
      username: "seeed",
      password: "",
      remember: true,
      deviceId: "5dbfb12400000000",
    });

    await store.getState().connect();
    expect(store.getState().state).toBe("desktop_opened");
    expect(ctl.launches[0].sessionId).toBe("seeed@5dbfb12400000000");
  });

  it("entering another address of a connected device reuses the session", async () => {
    useSessionsStore.getState().register(
      {
        host: "192.168.2.18",
        username: "seeed",
        password: "pw",
        deviceId: "5dbfb12400000000",
        displayName: "robotics",
      },
      DEVICE_V3,
    );

    const ctl = controllableService(DEVICE_V3);
    const store = createConnectionStore(() => ctl.service);
    store.getState().setForm({
      host: "100.114.170.49",
      username: "seeed",
      password: "pw",
      remember: true,
      deviceId: null,
    });

    await store.getState().connect();
    const s = store.getState();
    expect(s.state).toBe("idle");
    expect(s.notice).toContain("已作为「robotics」连接");
    expect(ctl.launches).toHaveLength(0); // no second desktop for one device
    expect(useSessionsStore.getState().order).toHaveLength(1); // still ONE tab
  });

  it("an unreachable address falls through to the next path", async () => {
    const ctl = controllableService(DEVICE_V3);
    const original = ctl.service.connect.bind(ctl.service);
    ctl.service.connect = async (input, opts) => {
      if (input.host === "192.168.2.18") {
        ctl.connectHosts.push(input.host);
        throw new ConnectionFailure("ssh_timeout");
      }
      return original(input, opts);
    };
    const mem = fakeMemory([savedDeviceFixture()]);
    const store = createConnectionStore(() => ctl.service, mem.gateway);
    store.setState({ savedDevices: [savedDeviceFixture()] });
    store.getState().setForm({
      host: "192.168.2.18",
      username: "seeed",
      password: "",
      remember: true,
      deviceId: "5dbfb12400000000",
    });

    await store.getState().connect();
    expect(store.getState().state).toBe("desktop_opened");
    expect(ctl.connectHosts).toEqual(["192.168.2.18", "100.114.170.49"]);
    // The winning path is what prepare/launch use.
    expect(ctl.launches[0].host).toBe("100.114.170.49");
    expect(ctl.launches[0].sessionId).toBe("seeed@5dbfb12400000000");
  });

  it("auth failure on one address falls through to the next path", async () => {
    const ctl = controllableService(DEVICE_V3);
    const original = ctl.service.connect.bind(ctl.service);
    ctl.service.connect = async (input, opts) => {
      if (input.host === "192.168.2.18") {
        ctl.connectHosts.push(input.host);
        throw new ConnectionFailure("auth_failed");
      }
      return original(input, opts);
    };
    const mem = fakeMemory([savedDeviceFixture()]);
    const store = createConnectionStore(() => ctl.service, mem.gateway);
    store.setState({ savedDevices: [savedDeviceFixture()] });
    store.getState().setForm({
      host: "192.168.2.18",
      username: "seeed",
      password: "",
      remember: true,
      deviceId: "5dbfb12400000000",
    });

    await store.getState().connect();
    expect(store.getState().state).toBe("desktop_opened");
    expect(ctl.connectHosts).toEqual(["192.168.2.18", "100.114.170.49"]);
  });

  it("paths are probed in parallel and ordered by lowest RTT first", async () => {
    probePathsMock.mockResolvedValueOnce([
      { address: "192.168.2.18", reachable: true, rttMs: 40 },
      { address: "100.114.170.49", reachable: true, rttMs: 5 },
    ]);
    const ctl = controllableService(DEVICE_V3);
    const mem = fakeMemory([savedDeviceFixture()]);
    const store = createConnectionStore(() => ctl.service, mem.gateway);
    store.setState({ savedDevices: [savedDeviceFixture()] });
    store.getState().setForm({
      host: "192.168.2.18",
      username: "seeed",
      password: "",
      remember: true,
      deviceId: "5dbfb12400000000",
    });

    await store.getState().connect();
    expect(store.getState().state).toBe("desktop_opened");
    // Lowest RTT (Tailscale) is tried first even though the form holds LAN.
    expect(ctl.connectHosts[0]).toBe("100.114.170.49");
    expect(probePathsMock).toHaveBeenCalledWith([
      "192.168.2.18",
      "100.114.170.49",
    ]);
  });

  it("every address failing surfaces the connect error", async () => {
    const ctl = controllableService(DEVICE_V3);
    ctl.setBehavior("auth_failed"); // fails on every candidate
    const mem = fakeMemory([savedDeviceFixture()]);
    const store = createConnectionStore(() => ctl.service, mem.gateway);
    store.setState({ savedDevices: [savedDeviceFixture()] });
    store.getState().setForm({
      host: "192.168.2.18",
      username: "seeed",
      password: "pw",
      remember: true,
      deviceId: "5dbfb12400000000",
    });

    await store.getState().connect();
    expect(store.getState().state).toBe("error");
    expect(store.getState().error?.code).toBe("auth_failed");
    expect(ctl.connectHosts).toEqual(["192.168.2.18", "100.114.170.49"]);
  });
});
