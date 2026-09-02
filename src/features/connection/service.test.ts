import { describe, expect, it } from "vitest";
import { MockConnectionService, delay, isAbortError } from "./service";
import { ConnectionInput, JetsonDevice } from "./types";

const input: ConnectionInput = {
  host: "192.168.100.164",
  username: "seeed",
  password: "s3cret-pw",
  remember: false,
};

describe("MockConnectionService", () => {
  it("resolves a fixture device on success without leaking the password", async () => {
    const svc = new MockConnectionService(() => 0);
    const seen: string[] = [];
    const outcome = await svc.connect(input, {
      scenario: "success",
      onProgress: (p) => seen.push(`${p.message} ${p.detail ?? ""}`),
    });
    expect(outcome.kind).toBe("device");
    const device = (outcome as { kind: "device"; device: JetsonDevice }).device;
    expect(device.host).toBe("192.168.100.164");
    expect(device.model).toBe("reComputer J501 mini");
    expect(device).not.toHaveProperty("password");
    expect(seen.join("\n")).not.toContain("s3cret-pw");
  });

  it("surfaces host_key_unknown and host_key_changed as outcomes", async () => {
    const svc = new MockConnectionService(() => 0);
    const unknown = await svc.connect(input, { scenario: "host_key_unknown" });
    expect(unknown).toMatchObject({ kind: "host_key_unknown" });

    const changed = await svc.connect(input, { scenario: "host_key_changed" });
    expect(changed).toMatchObject({ kind: "host_key_changed" });
  });

  it("throws the mapped failure per scenario", async () => {
    const svc = new MockConnectionService(() => 0);
    const cases = [
      ["ssh_timeout", "ssh_timeout"],
      ["auth_failed", "auth_failed"],
      ["not_jetson", "not_jetson"],
      ["provision_failed", "provision_failed"],
    ] as const;
    for (const [scenario, code] of cases) {
      await expect(svc.connect(input, { scenario })).rejects.toMatchObject({
        code,
      });
    }
  });

  it("aborts via signal", async () => {
    const svc = new MockConnectionService(() => 60_000);
    const ac = new AbortController();
    const p = svc.connect(input, { signal: ac.signal });
    ac.abort();
    await expect(p).rejects.toMatchObject({ name: "AbortError" });
  });
});

describe("delay / isAbortError", () => {
  it("delay rejects on abort", async () => {
    const ac = new AbortController();
    const p = delay(60_000, ac.signal);
    ac.abort();
    await expect(p).rejects.toMatchObject({ name: "AbortError" });
  });

  it("isAbortError recognizes AbortError only", () => {
    expect(
      isAbortError(Object.assign(new Error("x"), { name: "AbortError" })),
    ).toBe(true);
    expect(isAbortError(new Error("x"))).toBe(false);
  });
});