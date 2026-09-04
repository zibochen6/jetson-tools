import { describe, expect, it } from "vitest";
import {
  candidateAddresses,
  classifyAddress,
  deviceKey,
  pathLabel,
} from "./paths";

describe("classifyAddress", () => {
  it("classifies the CGNAT range (100.64.0.0/10) as tailscale", () => {
    expect(classifyAddress("100.114.170.49")).toBe("tailscale");
    expect(classifyAddress("100.64.0.1")).toBe("tailscale");
    expect(classifyAddress("100.94.85.115")).toBe("tailscale");
    expect(classifyAddress("100.127.255.254")).toBe("tailscale");
  });

  it("classifies everything else as lan", () => {
    expect(classifyAddress("192.168.2.18")).toBe("lan");
    expect(classifyAddress("10.0.0.5")).toBe("lan");
    expect(classifyAddress("172.16.3.4")).toBe("lan");
    // Outside the /10: 100.63 and 100.128 are NOT tailscale.
    expect(classifyAddress("100.63.0.1")).toBe("lan");
    expect(classifyAddress("100.128.0.1")).toBe("lan");
    expect(classifyAddress("jetson.local")).toBe("lan");
  });
});

describe("pathLabel", () => {
  it("renders kind + address", () => {
    expect(pathLabel("192.168.2.18")).toBe("LAN 192.168.2.18");
    expect(pathLabel("100.114.170.49")).toBe("Tailscale 100.114.170.49");
  });

  it("is empty without an address", () => {
    expect(pathLabel(null)).toBe("");
    expect(pathLabel(undefined)).toBe("");
    expect(pathLabel("")).toBe("");
  });
});

describe("candidateAddresses", () => {
  it("keeps the typed entry host first and dedupes", () => {
    expect(
      candidateAddresses("192.168.2.18", [
        "192.168.2.18",
        "100.114.170.49",
      ]),
    ).toEqual(["192.168.2.18", "100.114.170.49"]);
  });

  it("adds unknown known-paths after the entry host", () => {
    expect(
      candidateAddresses("10.0.0.9", ["192.168.2.18", "100.114.170.49"]),
    ).toEqual(["10.0.0.9", "192.168.2.18", "100.114.170.49"]);
  });

  it("tolerates blanks", () => {
    expect(candidateAddresses(" 10.0.0.9 ", ["", "  "])).toEqual(["10.0.0.9"]);
    expect(candidateAddresses("", undefined)).toEqual([]);
  });
});

describe("deviceKey", () => {
  it("uses username@deviceId when a machine-id is known", () => {
    expect(deviceKey("seeed", "5dbfb124", "192.168.2.18")).toBe(
      "seeed@5dbfb124",
    );
  });

  it("falls back to username@host for legacy devices", () => {
    expect(deviceKey("seeed", null, "192.168.2.18")).toBe(
      "seeed@192.168.2.18",
    );
    expect(deviceKey("seeed", "  ", "192.168.2.18")).toBe(
      "seeed@192.168.2.18",
    );
  });
});
