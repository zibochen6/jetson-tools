import { describe, expect, it } from "vitest";
import {
  isValidHost,
  isValidHostname,
  isValidIpv4,
  isValidPassword,
  isValidUsername,
  validateConnectionForm,
} from "./validation";

describe("IPv4 validation", () => {
  it("accepts valid IPv4", () => {
    expect(isValidIpv4("192.168.100.164")).toBe(true);
    expect(isValidIpv4("0.0.0.0")).toBe(true);
    expect(isValidIpv4("255.255.255.255")).toBe(true);
  });

  it("rejects invalid IPv4", () => {
    expect(isValidIpv4("256.1.1.1")).toBe(false);
    expect(isValidIpv4("1.2.3")).toBe(false);
    expect(isValidIpv4("1.2.3.4.5")).toBe(false);
    expect(isValidIpv4("192.168.001.1")).toBe(false);
  });
});

describe("hostname validation", () => {
  it("accepts hostnames", () => {
    expect(isValidHostname("jetson.local")).toBe(true);
    expect(isValidHostname("jetson")).toBe(true);
    expect(isValidHostname("recomputer-j501")).toBe(true);
    expect(isValidHostname("my-jetson.example.com")).toBe(true);
  });

  it("rejects bad hostnames", () => {
    expect(isValidHostname("has space")).toBe(false);
    expect(isValidHostname("-leading")).toBe(false);
    expect(isValidHostname("trailing-")).toBe(false);
  });
});

describe("isValidHost", () => {
  it("accepts IPv4 or hostname, rejects empty/invalid", () => {
    expect(isValidHost("192.168.1.100")).toBe(true);
    expect(isValidHost("jetson.local")).toBe(true);
    expect(isValidHost("")).toBe(false);
    expect(isValidHost("not valid")).toBe(false);
  });
});

describe("username/password validation", () => {
  it("requires non-empty username and password", () => {
    expect(isValidUsername("")).toBe(false);
    expect(isValidUsername("   ")).toBe(false);
    expect(isValidUsername("seeed")).toBe(true);
    expect(isValidPassword("")).toBe(false);
    expect(isValidPassword("x")).toBe(true);
  });
});

describe("validateConnectionForm", () => {
  it("reports per-field errors for an empty form", () => {
    const errors = validateConnectionForm({
      host: "",
      username: "",
      password: "",
      remember: false,
    });
    expect(errors.host).toBeTruthy();
    expect(errors.username).toBeTruthy();
    expect(errors.password).toBeTruthy();
  });

  it("accepts a valid form with no errors", () => {
    const errors = validateConnectionForm({
      host: "192.168.1.1",
      username: "u",
      password: "p",
      remember: false,
    });
    expect(Object.keys(errors)).toHaveLength(0);
  });

  it("blank password is valid when a stored password matches the device", () => {
    const saved = {
      host: "192.168.1.1",
      username: "u",
      hasPassword: true,
    };
    const errors = validateConnectionForm(
      {
        host: "192.168.1.1",
        username: "u",
        password: "",
        remember: true,
      },
      saved,
    );
    expect(Object.keys(errors)).toHaveLength(0);
    expect(errors.password).toBeUndefined();
  });

  it("requires a password when the saved device does not match", () => {
    const saved = { host: "10.0.0.2", username: "other", hasPassword: true };
    const errors = validateConnectionForm(
      {
        host: "192.168.1.1",
        username: "u",
        password: "",
        remember: false,
      },
      saved,
    );
    expect(errors.password).toBeTruthy();
  });

  it("requires a password when the device matches but no password is stored", () => {
    const saved = { host: "192.168.1.1", username: "u", hasPassword: false };
    const errors = validateConnectionForm(
      {
        host: "192.168.1.1",
        username: "u",
        password: "",
        remember: true,
      },
      saved,
    );
    expect(errors.password).toBeTruthy();
  });

  it("no saved device keeps the old strict password rule", () => {
    const errors = validateConnectionForm({
      host: "192.168.1.1",
      username: "u",
      password: "",
      remember: false,
    });
    expect(errors.password).toBeTruthy();
  });
});