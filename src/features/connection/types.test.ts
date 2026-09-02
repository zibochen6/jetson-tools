import { describe, expect, it } from "vitest";
import { ConnectionFailure, describeError } from "./types";

describe("describeError", () => {
  it("returns non-empty copy for every error code", () => {
    const codes = [
      "ssh_timeout",
      "auth_failed",
      "not_jetson",
      "provision_failed",
      "rdp_failed",
      "unknown",
    ] as const;
    for (const code of codes) {
      const e = describeError(code);
      expect(e.title.length).toBeGreaterThan(0);
      expect(e.code).toBe(code);
    }
  });

  it("maps ssh timeout to network checks", () => {
    const e = describeError("ssh_timeout");
    expect(e.title).toBe("Couldn't reach this Jetson");
    expect(e.suggestions.length).toBeGreaterThan(0);
  });

  it("maps auth failure to credential guidance", () => {
    expect(describeError("auth_failed").title).toBe("Authentication failed");
  });

  it("maps not-a-jetson distinctly", () => {
    expect(describeError("not_jetson").title).toBe(
      "This device doesn't appear to be an NVIDIA Jetson.",
    );
  });

  it("has a graceful unknown fallback", () => {
    expect(describeError("unknown").title).toBe("Something went wrong.");
  });
});

describe("ConnectionFailure", () => {
  it("carries its error code and is an Error", () => {
    const err = new ConnectionFailure("not_jetson");
    expect(err.code).toBe("not_jetson");
    expect(err).toBeInstanceOf(Error);
  });
});