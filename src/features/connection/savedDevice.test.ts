import { beforeEach, describe, expect, it, vi } from "vitest";

const { invokeMock } = vi.hoisted(() => ({ invokeMock: vi.fn() }));
vi.mock("@tauri-apps/api/core", () => ({
  invoke: invokeMock,
  Channel: class {
    onmessage: ((event: unknown) => void) | null = null;
  },
}));

import { TauriDeviceMemoryGateway } from "./savedDevice";

describe("TauriDeviceMemoryGateway", () => {
  beforeEach(() => {
    invokeMock.mockReset();
  });

  it("loadAll returns typed devices in backend order", async () => {
    invokeMock.mockResolvedValue([{
      deviceId: "5dbfb124",
      username: "seeed",
      displayName: "robotics",
      paths: [{ kind: "lan", address: "192.168.2.18" }],
      lastUsedPath: "192.168.2.18",
      hasPassword: true,
    }]);
    const out = await new TauriDeviceMemoryGateway().loadAll();
    expect(invokeMock).toHaveBeenCalledWith("get_remembered_devices");
    expect(out).toEqual([{
      deviceId: "5dbfb124",
      username: "seeed",
      displayName: "robotics",
      paths: [{ kind: "lan", address: "192.168.2.18" }],
      lastUsedPath: "192.168.2.18",
      hasPassword: true,
    }]);
  });

  it("loadAll returns an empty list when the backend has no memory", async () => {
    invokeMock.mockResolvedValue([]);
    expect(await new TauriDeviceMemoryGateway().loadAll()).toEqual([]);
  });

  it("loadAll degrades to an empty list on any failure (browser dev / io error)", async () => {
    invokeMock.mockRejectedValue(new Error("not in Tauri"));
    expect(await new TauriDeviceMemoryGateway().loadAll()).toEqual([]);
  });

  it("save forwards the v3 identity (deviceId + name + paths) once", async () => {
    invokeMock.mockResolvedValue(undefined);
    await new TauriDeviceMemoryGateway().save({
      deviceId: "5dbfb124",
      username: "seeed",
      displayName: "robotics",
      paths: [
        { kind: "lan", address: "192.168.2.18" },
        { kind: "tailscale", address: "100.114.170.49" },
      ],
      entryHost: "192.168.2.18",
      password: "secret",
    });
    expect(invokeMock).toHaveBeenCalledWith("remember_device", {
      input: {
        deviceId: "5dbfb124",
        username: "seeed",
        displayName: "robotics",
        paths: [
          { kind: "lan", address: "192.168.2.18" },
          { kind: "tailscale", address: "100.114.170.49" },
        ],
        entryHost: "192.168.2.18",
        password: "secret",
      },
    });
  });

  it("forget invokes the delete command by identity", async () => {
    invokeMock.mockResolvedValue(undefined);
    await new TauriDeviceMemoryGateway().forget({
      deviceId: "5dbfb124",
      host: "192.168.2.18",
      username: "seeed",
    });
    expect(invokeMock).toHaveBeenCalledWith("forget_remembered_device", {
      deviceId: "5dbfb124",
      host: "192.168.2.18",
      username: "seeed",
    });
  });
});
