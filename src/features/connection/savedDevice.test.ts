import { beforeEach, describe, expect, it, vi } from "vitest";

const { invokeMock } = vi.hoisted(() => ({ invokeMock: vi.fn() }));
vi.mock("@tauri-apps/api/core", () => ({
  invoke: invokeMock,
}));

import { TauriDeviceMemoryGateway } from "./savedDevice";

describe("TauriDeviceMemoryGateway", () => {
  beforeEach(() => {
    invokeMock.mockReset();
  });

  it("load returns the typed device status", async () => {
    invokeMock.mockResolvedValue({
      host: "192.168.100.164",
      username: "seeed",
      hasPassword: true,
    });
    const out = await new TauriDeviceMemoryGateway().load();
    expect(invokeMock).toHaveBeenCalledWith("get_remembered_device");
    expect(out).toEqual({
      host: "192.168.100.164",
      username: "seeed",
      hasPassword: true,
    });
  });

  it("load returns null when the backend has no memory", async () => {
    invokeMock.mockResolvedValue(null);
    expect(await new TauriDeviceMemoryGateway().load()).toBeNull();
  });

  it("load degrades to null on any failure (browser dev / io error)", async () => {
    invokeMock.mockRejectedValue(new Error("not in Tauri"));
    expect(await new TauriDeviceMemoryGateway().load()).toBeNull();
  });

  it("save forwards identity + password once", async () => {
    invokeMock.mockResolvedValue(undefined);
    await new TauriDeviceMemoryGateway().save({
      host: "h",
      username: "u",
      password: "secret",
    });
    expect(invokeMock).toHaveBeenCalledWith("remember_device", {
      host: "h",
      username: "u",
      password: "secret",
    });
  });

  it("forget invokes the delete command", async () => {
    invokeMock.mockResolvedValue(undefined);
    await new TauriDeviceMemoryGateway().forget();
    expect(invokeMock).toHaveBeenCalledWith("forget_remembered_device");
  });
});