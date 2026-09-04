import { beforeEach, describe, expect, it, vi } from "vitest";

const { invokeMock } = vi.hoisted(() => ({ invokeMock: vi.fn() }));
vi.mock("@tauri-apps/api/core", () => ({
  invoke: invokeMock,
  Channel: class {
    onmessage: unknown;
    constructor() {
      this.onmessage = undefined;
    }
  },
}));

import { TauriConnectionService, TauriSessionService } from "./tauriService";
import { ConnectionInput } from "./types";

const input: ConnectionInput = {
  host: "192.168.100.164",
  username: "seeed",
  password: "pw",
  remember: false,
};

describe("TauriConnectionService", () => {
  beforeEach(() => {
    invokeMock.mockReset();
  });

  it("maps a device result", async () => {
    invokeMock.mockResolvedValue({
      kind: "device",
      device: { host: "192.168.100.164", model: "reComputer J501 mini" },
    });
    const out = await new TauriConnectionService().connect(input, {});
    expect(out).toEqual({
      kind: "device",
      device: expect.objectContaining({ host: "192.168.100.164" }),
    });
  });

  it("maps host key unknown", async () => {
    invokeMock.mockResolvedValue({
      kind: "hostKeyUnknown",
      key: { host: "h", port: 22, algorithm: "ssh-ed25519", fingerprint: "SHA256:x" },
    });
    const out = await new TauriConnectionService().connect(input, {});
    expect(out.kind).toBe("host_key_unknown");
  });

  it("maps auth failure -> auth_failed", async () => {
    invokeMock.mockRejectedValue({ code: "AUTHENTICATION_FAILED", message: "x" });
    await expect(new TauriConnectionService().connect(input, {})).rejects.toMatchObject({
      code: "auth_failed",
    });
  });

  it("maps timeout -> ssh_timeout", async () => {
    invokeMock.mockRejectedValue({ code: "SSH_TIMEOUT", message: "x" });
    await expect(new TauriConnectionService().connect(input, {})).rejects.toMatchObject({
      code: "ssh_timeout",
    });
  });

  it("maps connection refused -> ssh_timeout", async () => {
    invokeMock.mockRejectedValue({ code: "CONNECTION_REFUSED", message: "x" });
    await expect(new TauriConnectionService().connect(input, {})).rejects.toMatchObject({
      code: "ssh_timeout",
    });
  });

  it("maps not-a-jetson -> not_jetson", async () => {
    invokeMock.mockRejectedValue({ code: "NOT_A_JETSON", message: "x" });
    await expect(new TauriConnectionService().connect(input, {})).rejects.toMatchObject({
      code: "not_jetson",
    });
  });

  it("maps detection parse -> detection_failed", async () => {
    invokeMock.mockRejectedValue({ code: "DETECTION_PARSE_FAILED", message: "x" });
    await expect(new TauriConnectionService().connect(input, {})).rejects.toMatchObject({
      code: "detection_failed",
    });
  });

  it("invokes probe_device with host/username/password", async () => {
    invokeMock.mockResolvedValue({ kind: "device", device: { host: "h" } });
    await new TauriConnectionService().connect(input, {});
    expect(invokeMock).toHaveBeenCalledWith("probe_device", {
      input: { host: "192.168.100.164", port: 22, username: "seeed", deviceId: null, password: "pw" },
      hostKeyDecision: null,
    });
  });

  it("sends null for an empty password (backend resolves stored secret)", async () => {
    invokeMock.mockResolvedValue({ kind: "device", device: { host: "h" } });
    await new TauriConnectionService().connect(
      { ...input, password: "" },
      {},
    );
    expect(invokeMock).toHaveBeenCalledWith("probe_device", {
      input: { host: "192.168.100.164", port: 22, username: "seeed", deviceId: null, password: null },
      hostKeyDecision: null,
    });
  });

  it("maps saved password missing -> saved_password_missing", async () => {
    invokeMock.mockRejectedValue({
      code: "SAVED_PASSWORD_MISSING",
      message: "x",
    });
    await expect(new TauriConnectionService().connect(input, {})).rejects.toMatchObject({
      code: "saved_password_missing",
    });
  });

  it("prepare sends null for an empty password too", async () => {
    invokeMock.mockResolvedValue({
      kind: "ready",
      wasAlreadyReady: true,
      environment: {},
    });
    const svc = new TauriConnectionService();
    const onEvent = vi.fn();
    await svc.prepare({ ...input, password: "" }, { onEvent });
    expect(invokeMock).toHaveBeenCalledWith(
      "prepare_remote_desktop",
      expect.objectContaining({
        input: expect.objectContaining({
          host: "192.168.100.164",
          username: "seeed",
          password: null,
        }),
      }),
    );
  });

  it("frontend always sends the typed host; tunnel routing is backend-side (KI-021)", async () => {
    vi.stubEnv("VITE_JR_SSH_PORT", "2222");
    vi.resetModules();
    try {
      const { TauriConnectionService: Svc } = await import("./tauriService");
      invokeMock.mockResolvedValue({ kind: "device", device: {} });
      await new Svc().connect(input, {});
      expect(invokeMock).toHaveBeenLastCalledWith("probe_device", {
        input: { host: "192.168.100.164", port: 22, username: "seeed", deviceId: null, password: "pw" },
        hostKeyDecision: null,
      });
      invokeMock.mockResolvedValue({ kind: "opened" });
      await new Svc().launch({
        host: "192.168.100.164",
        username: "seeed",
        password: "pw",
      });
      expect(invokeMock).toHaveBeenLastCalledWith("launch_remote_desktop", {
        request: {
          host: "192.168.100.164",
          username: "seeed",
          deviceId: null,
          password: "pw",
        },
      });
    } finally {
      vi.unstubAllEnvs();
      vi.resetModules();
    }
  });

  it("launch returns the typed RDP result", async () => {
    invokeMock.mockResolvedValue({ kind: "opened" });
    const out = await new TauriConnectionService().launch({
      host: "192.168.100.164",
      username: "seeed",
      password: "pw",
    });
    expect(out).toEqual({ kind: "opened" });
    expect(invokeMock).toHaveBeenCalledWith("launch_remote_desktop", {
      request: {
        host: "192.168.100.164",
        username: "seeed",
        deviceId: null,
        password: "pw",
      },
    });
  });

  it("keyed launch defaults to taking focus", async () => {
    invokeMock.mockResolvedValue({ kind: "opened" });
    await new TauriConnectionService().launch(
      {
        host: "192.168.100.164",
        username: "seeed",
        password: "pw",
      },
      { sessionId: "seeed@192.168.100.164" },
    );
    expect(invokeMock).toHaveBeenCalledWith("launch_session", {
      sessionId: "seeed@192.168.100.164",
      focusOnLaunch: true,
      request: {
        host: "192.168.100.164",
        username: "seeed",
        deviceId: null,
        password: "pw",
      },
    });
  });

  it("session recovery can launch without taking focus", async () => {
    invokeMock.mockResolvedValue({ kind: "opened" });
    await new TauriSessionService().launch(
      "seeed@192.168.100.164",
      {
        host: "192.168.100.164",
        username: "seeed",
        password: "pw",
      },
      { focusOnLaunch: false },
    );
    expect(invokeMock).toHaveBeenCalledWith("launch_session", {
      sessionId: "seeed@192.168.100.164",
      focusOnLaunch: false,
      request: {
        host: "192.168.100.164",
        username: "seeed",
        deviceId: null,
        password: "pw",
      },
    });
  });

  it("launch sends null password (backend resolves stored secret)", async () => {
    invokeMock.mockResolvedValue({ kind: "opened" });
    await new TauriConnectionService().launch({
      host: "192.168.100.164",
      username: "seeed",
      password: "",
    });
    expect(invokeMock).toHaveBeenCalledWith("launch_remote_desktop", {
      request: {
        host: "192.168.100.164",
        username: "seeed",
        deviceId: null,
        password: null,
      },
    });
  });

  it("maps RDP_CLIENT_NOT_FOUND -> rdp_client_missing", async () => {
    invokeMock.mockRejectedValue({
      code: "RDP_CLIENT_NOT_FOUND",
      message: "x",
    });
    await expect(
      new TauriConnectionService().launch({
        host: "h",
        username: "u",
        password: "p",
      }),
    ).rejects.toMatchObject({ code: "rdp_client_missing" });
  });

  it("maps RDP_LAUNCH_FAILED -> rdp_failed", async () => {
    invokeMock.mockRejectedValue({ code: "RDP_LAUNCH_FAILED", message: "x" });
    await expect(
      new TauriConnectionService().launch({
        host: "h",
        username: "u",
        password: "p",
      }),
    ).rejects.toMatchObject({ code: "rdp_failed" });
  });

  it("maps RDP_PASSWORD_MISSING -> saved_password_missing", async () => {
    invokeMock.mockRejectedValue({ code: "RDP_PASSWORD_MISSING", message: "x" });
    await expect(
      new TauriConnectionService().launch({
        host: "h",
        username: "u",
        password: "",
      }),
    ).rejects.toMatchObject({ code: "saved_password_missing" });
  });

  it("status invokes rdp_status and passes through", async () => {
    invokeMock.mockResolvedValue({ kind: "running" });
    const status = await new TauriConnectionService().status();
    expect(status).toEqual({ kind: "running" });
    expect(invokeMock).toHaveBeenCalledWith("rdp_status");
  });
});

describe("identity-v3 wire shape", () => {
  beforeEach(() => {
    invokeMock.mockReset();
  });

  it("connect forwards the deviceId of a remembered v3 device", async () => {
    invokeMock.mockResolvedValue({ kind: "device", device: { host: "h" } });
    await new TauriConnectionService().connect(
      { ...input, deviceId: "5dbfb124" },
      {},
    );
    expect(invokeMock).toHaveBeenCalledWith("probe_device", {
      input: {
        host: "192.168.100.164",
        port: 22,
        username: "seeed",
        deviceId: "5dbfb124",
        password: "pw",
      },
      hostKeyDecision: null,
    });
  });

  it("launch forwards the deviceId (tunnel/session key)", async () => {
    invokeMock.mockResolvedValue({ kind: "opened" });
    await new TauriConnectionService().launch(
      {
        host: "192.168.2.18",
        username: "seeed",
        password: "pw",
        deviceId: "5dbfb124",
      },
      { sessionId: "seeed@5dbfb124" },
    );
    expect(invokeMock).toHaveBeenCalledWith("launch_session", {
      sessionId: "seeed@5dbfb124",
      focusOnLaunch: true,
      request: {
        host: "192.168.2.18",
        username: "seeed",
        deviceId: "5dbfb124",
        password: "pw",
      },
    });
  });
});

describe("probeDevicePaths", () => {
  beforeEach(() => {
    invokeMock.mockReset();
  });

  it("probes every unique address once", async () => {
    invokeMock.mockResolvedValue([
      { address: "192.168.2.18", reachable: true, rttMs: 3 },
    ]);
    const out = await (await import("./tauriService")).probeDevicePaths([
      "192.168.2.18",
      "192.168.2.18",
      " 100.114.170.49 ",
      "",
    ]);
    expect(invokeMock).toHaveBeenCalledWith("probe_device_paths", {
      addresses: ["192.168.2.18", " 100.114.170.49 "],
    });
    expect(out).toHaveLength(1);
  });

  it("degrades to an empty list outside Tauri (caller keeps input order)", async () => {
    invokeMock.mockRejectedValue(new Error("not in Tauri"));
    const out = await (await import("./tauriService")).probeDevicePaths([
      "192.168.2.18",
    ]);
    expect(out).toEqual([]);
  });

  it("skips the IPC round-trip for an empty candidate list", async () => {
    const out = await (await import("./tauriService")).probeDevicePaths([]);
    expect(out).toEqual([]);
    expect(invokeMock).not.toHaveBeenCalled();
  });
});
