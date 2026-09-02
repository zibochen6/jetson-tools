import { describe, expect, it } from "vitest";
import { ConnectionService } from "../features/connection/service";
import { DeviceMemoryGateway } from "../features/connection/savedDevice";
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

const DEVICE: JetsonDevice = { host: "", model: "reComputer J501 mini" };

const ENV: RemoteEnvironmentReport = {
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

type Behavior =
  | "success"
  | "auth_failed"
  | "host_key_unknown"
  | "sudo_denied"
  | "provision_failed"
  | "verification_failed"
  | "launch_failed";

function controllableService() {
  let behavior: Behavior = "success";
  let rdpRunning = false;
  let rdpExited = false;

  const service: ConnectionService = {
    async connect(input, opts): Promise<ConnectOutcome> {
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
      return { kind: "device", device: { ...DEVICE, host: input.host } };
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
    async launch(): Promise<RdpLaunchResult> {
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
  };
}

/** In-memory device-memory gateway; records calls for assertions. */
function fakeMemory(initial: NonNullable<Awaited<ReturnType<DeviceMemoryGateway["load"]>>> | null = null) {
  const calls: string[] = [];
  let memory: Awaited<ReturnType<DeviceMemoryGateway["load"]>> = initial;
  const gateway: DeviceMemoryGateway = {
    async load() {
      calls.push("load");
      return memory;
    },
    async save(input) {
      calls.push(`save:${input.host}:${input.username}:${input.password}`);
      memory = {
        host: input.host,
        username: input.username,
        hasPassword: true,
      };
    },
    async forget() {
      calls.push("forget");
      memory = null;
    },
  };
  return {
    gateway,
    calls,
    setMemory: (m: Awaited<ReturnType<DeviceMemoryGateway["load"]>>) => {
      memory = m;
    },
  };
}

function savedDeviceFixture() {
  return {
    host: "192.168.100.164",
    username: "seeed",
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
    expect(mem.calls).toContain("save:192.168.100.164:seeed:secret");
    expect(store.getState().savedDevice).toEqual(savedDeviceFixture());
  });

  it("blank password with remembered device does not rewrite memory", async () => {
    const mem = fakeMemory(savedDeviceFixture());
    const store = createConnectionStore(
      () => controllableService().service,
      mem.gateway,
    );
    store.setState({ savedDevice: savedDeviceFixture() });
    store.getState().setForm({
      host: "192.168.100.164",
      username: "seeed",
      password: "",
      remember: true,
    });

    await store.getState().connect();
    expect(store.getState().state).toBe("desktop_opened");
    expect(mem.calls).not.toContain("save:192.168.100.164:seeed:");
    expect(store.getState().savedDevice).toEqual(savedDeviceFixture());
  });

  it("unchecking remember on the remembered device forgets it", async () => {
    const mem = fakeMemory(savedDeviceFixture());
    const store = createConnectionStore(
      () => controllableService().service,
      mem.gateway,
    );
    store.getState().setForm({ ...filledForm(), remember: false });
    // seed the store state so the match check fires
    store.setState({ savedDevice: savedDeviceFixture() });

    await store.getState().connect();
    expect(store.getState().state).toBe("desktop_opened");
    expect(mem.calls).toContain("forget");
    expect(store.getState().savedDevice).toBeNull();
  });

  it("unchecking remember on a DIFFERENT device keeps the memory", async () => {
    const mem = fakeMemory(savedDeviceFixture());
    const store = createConnectionStore(
      () => controllableService().service,
      mem.gateway,
    );
    store.getState().setForm({
      host: "192.168.1.99",
      username: "other",
      password: "pw",
      remember: false,
    });
    store.setState({ savedDevice: savedDeviceFixture() });

    await store.getState().connect();
    expect(store.getState().state).toBe("desktop_opened");
    expect(mem.calls).not.toContain("forget");
    expect(store.getState().savedDevice).toEqual(savedDeviceFixture());
  });

  it("memory save failure never breaks the connection flow", async () => {
    const mem = fakeMemory();
    const failing: DeviceMemoryGateway = {
      load: mem.gateway.load,
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
    const mem = fakeMemory(savedDeviceFixture());
    const store = createConnectionStore(
      () => controllableService().service,
      mem.gateway,
    );

    await store.getState().initRemembered();
    await flush(); // auto-connect resolves asynchronously
    const s = store.getState();
    expect(s.form).toMatchObject({
      host: "192.168.100.164",
      username: "seeed",
      password: "",
      remember: true,
    });
    expect(s.state).toBe("desktop_opened");
  });

  it("initRemembered without a password prefills but stays idle", async () => {
    const mem = fakeMemory({
      host: "192.168.100.164",
      username: "seeed",
      hasPassword: false,
    });
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
    const mem = fakeMemory(savedDeviceFixture());
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

  it("forgetDevice clears memory and unchecks remember", async () => {
    const mem = fakeMemory(savedDeviceFixture());
    const store = createConnectionStore(
      () => controllableService().service,
      mem.gateway,
    );
    await store.getState().initRemembered();

    await store.getState().forgetDevice();
    expect(mem.calls).toContain("forget");
    expect(store.getState().savedDevice).toBeNull();
    expect(store.getState().form.remember).toBe(false);
  });
});