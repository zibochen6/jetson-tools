import { describe, expect, it, vi } from "vitest";
import {
  createSessionsStore,
  sessionPathLabel,
  SessionDesktopGateway,
} from "./sessionsStore";
import { RdpLaunchResult, RdpStatus } from "../features/connection/types";
import { SessionStatusEntry } from "../features/connection/tauriService";

interface FakeGateway extends SessionDesktopGateway {
  calls: string[];
  statuses: Map<string, RdpStatus>;
  launchShouldFail: boolean;
}

function fakeGateway(): FakeGateway {
  const gw: FakeGateway = {
    calls: [],
    statuses: new Map(),
    launchShouldFail: false,
    async launch(sessionId, _input, options): Promise<RdpLaunchResult> {
      gw.calls.push(`launch:${sessionId}:${options.focusOnLaunch}`);
      if (gw.launchShouldFail) throw new Error("nope");
      gw.statuses.set(sessionId, { kind: "running" });
      return { kind: "opened" };
    },
    async focus(sessionId) {
      gw.calls.push(`focus:${sessionId ?? "null"}`);
    },
    async close(sessionId) {
      gw.calls.push(`close:${sessionId}`);
      gw.statuses.delete(sessionId);
    },
    async allStatuses(): Promise<SessionStatusEntry[]> {
      gw.calls.push("allStatuses");
      return [...gw.statuses.entries()].map(([sessionId, status]) => ({
        sessionId,
        status,
      }));
    },
  };
  return gw;
}

const A = { host: "192.168.1.31", username: "seeed", password: "pw-a" };
const B = { host: "192.168.1.42", username: "seeed", password: "pw-b" };

async function flush(n = 10) {
  for (let i = 0; i < n; i++) await Promise.resolve();
}

describe("sessionsStore (multi-device, V0.4)", () => {
  it("register adds a tab and focuses it", () => {
    const gw = fakeGateway();
    const store = createSessionsStore(gw);
    store.getState().register(A, { host: A.host, hostname: "orin" });

    const s = store.getState();
    expect(s.order).toEqual(["seeed@192.168.1.31"]);
    expect(s.activeId).toBe("seeed@192.168.1.31");
    expect(s.sessions["seeed@192.168.1.31"].phase).toBe("running");
  });

  it("registering a second device keeps the first tab (multi-device)", () => {
    const store = createSessionsStore(fakeGateway());
    store.getState().register(A, null);
    store.getState().register(B, null);

    const s = store.getState();
    expect(s.order).toEqual(["seeed@192.168.1.31", "seeed@192.168.1.42"]);
    expect(s.activeId).toBe("seeed@192.168.1.42");
  });

  it("focusTab on a running session switches without reconnecting", () => {
    const gw = fakeGateway();
    const store = createSessionsStore(gw);
    store.getState().register(A, null);
    store.getState().register(B, null);

    store.getState().focusTab("seeed@192.168.1.31");
    expect(store.getState().activeId).toBe("seeed@192.168.1.31");
    // backend focus only — NO relaunch
    expect(gw.calls).toContain("focus:seeed@192.168.1.31");
    expect(gw.calls.filter((c) => c.startsWith("launch:"))).toEqual([]);
  });

  it("focusTab on a ready session relaunches the desktop", async () => {
    const gw = fakeGateway();
    const store = createSessionsStore(gw);
    store.getState().register(A, null);
    gw.statuses.set("seeed@192.168.1.31", { kind: "notRunning" });
    await store.getState().pollStatuses();
    expect(store.getState().sessions["seeed@192.168.1.31"].phase).toBe("ready");
    expect(store.getState().activeId).toBeNull(); // stale view hidden

    store.getState().focusTab("seeed@192.168.1.31");
    await flush();
    expect(store.getState().sessions["seeed@192.168.1.31"].phase).toBe("running");
    expect(store.getState().activeId).toBe("seeed@192.168.1.31");
    expect(gw.calls).toContain("launch:seeed@192.168.1.31:true");
  });

  it("showOverview hides every native view but keeps sessions alive", () => {
    const gw = fakeGateway();
    const store = createSessionsStore(gw);
    store.getState().register(A, null);

    store.getState().showOverview();
    expect(store.getState().activeId).toBeNull();
    expect(gw.calls).toContain("focus:null");
    expect(Object.keys(store.getState().sessions)).toHaveLength(1);
  });

  it("closeTab closes the backend session and focuses the next live one", () => {
    const gw = fakeGateway();
    const store = createSessionsStore(gw);
    store.getState().register(A, null);
    store.getState().register(B, null);

    store.getState().closeTab("seeed@192.168.1.42");
    const s = store.getState();
    expect(s.order).toEqual(["seeed@192.168.1.31"]);
    expect(s.activeId).toBe("seeed@192.168.1.31");
    expect(gw.calls).toContain("close:seeed@192.168.1.42");
    expect(gw.calls).toContain("focus:seeed@192.168.1.31");
  });

  it("closeTab on the only session yields the screen back to the overview (KI-035)", () => {
    const gw = fakeGateway();
    const store = createSessionsStore(gw);
    store.getState().register(A, null);
    expect(store.getState().activeId).toBe("seeed@192.168.1.31");

    store.getState().closeTab("seeed@192.168.1.31");
    const s = store.getState();
    expect(s.order).toEqual([]);
    expect(s.activeId).toBeNull();
    expect(gw.calls).toContain("close:seeed@192.168.1.31");
    // Without this the backend never unmounts the native desktop view and the
    // last frame stays on screen over the webview home forever.
    expect(gw.calls).toContain("focus:null");
  });

  it("closeTab of a background tab leaves the focused desktop untouched (KI-035)", () => {
    const gw = fakeGateway();
    const store = createSessionsStore(gw);
    store.getState().register(A, null);
    store.getState().register(B, null);
    store.getState().focusTab("seeed@192.168.1.31");
    gw.calls.length = 0;

    store.getState().closeTab("seeed@192.168.1.42");
    expect(store.getState().activeId).toBe("seeed@192.168.1.31");
    expect(gw.calls).toContain("close:seeed@192.168.1.42");
    // The on-screen session must not be disturbed by closing a background tab.
    expect(gw.calls).not.toContain("focus:null");
    expect(gw.calls).not.toContain("focus:seeed@192.168.1.31");
  });

  it("clean backend exit marks the tab ready and returns to overview", async () => {
    const gw = fakeGateway();
    const store = createSessionsStore(gw);
    store.getState().register(A, null);

    gw.statuses.set("seeed@192.168.1.31", {
      kind: "exited",
      exitCode: 0,
      error: null,
    });
    await store.getState().pollStatuses();

    const s = store.getState();
    expect(s.sessions["seeed@192.168.1.31"].phase).toBe("ready");
    expect(s.activeId).toBeNull();
  });

  it("errored exit triggers bounded auto-relaunch (legacy parity)", async () => {
    vi.useFakeTimers();
    try {
      const gw = fakeGateway();
      const store = createSessionsStore(gw);
      store.getState().register(A, null);

      gw.statuses.set("seeed@192.168.1.31", {
        kind: "exited",
        exitCode: 1,
        error: "connect failed",
      });
      await store.getState().pollStatuses();
      expect(gw.calls.filter((c) => c.startsWith("launch:"))).toHaveLength(0);

      await vi.advanceTimersByTimeAsync(2100);
      await flush();
      // one auto-relaunch happened
      expect(gw.calls).toContain("launch:seeed@192.168.1.31:false");
      // gateway "relaunch" succeeded → running again
      expect(store.getState().sessions["seeed@192.168.1.31"].phase).toBe(
        "running",
      );
    } finally {
      vi.useRealTimers();
    }
  });

  it("background recovery never steals focus from another device", async () => {
    vi.useFakeTimers();
    try {
      const gw = fakeGateway();
      const store = createSessionsStore(gw);
      store.getState().register(B, null);
      store.getState().register(A, null); // A is the foreground device.
      gw.statuses.set("seeed@192.168.1.31", { kind: "running" });
      gw.statuses.set("seeed@192.168.1.42", {
        kind: "exited",
        exitCode: 1,
        error: "connect failed",
      });

      await store.getState().pollStatuses();
      await vi.advanceTimersByTimeAsync(2100);
      await flush();

      expect(gw.calls).toContain("launch:seeed@192.168.1.42:false");
      expect(store.getState().activeId).toBe("seeed@192.168.1.31");
      expect(gw.calls).not.toContain("focus:seeed@192.168.1.42");
      expect(store.getState().sessions["seeed@192.168.1.31"].phase).toBe(
        "running",
      );
    } finally {
      vi.useRealTimers();
    }
  });

  it("vanished backend session becomes re-openable (ready)", async () => {
    const gw = fakeGateway();
    const store = createSessionsStore(gw);
    store.getState().register(A, null);

    gw.statuses.clear(); // backend lost the session (crash / app restart)
    await store.getState().pollStatuses();
    expect(store.getState().sessions["seeed@192.168.1.31"].phase).toBe("ready");
  });

  it("poll is a no-op without sessions and survives gateway failure", async () => {
    const broken: SessionDesktopGateway = {
      launch: async () => ({ kind: "opened" }),
      focus: async () => {},
      close: async () => {},
      allStatuses: async () => {
        throw new Error("no tauri");
      },
    };
    const store = createSessionsStore(broken);
    await store.getState().pollStatuses(); // no sessions → early return
    store.getState().register(A, null);
    await store.getState().pollStatuses(); // gateway throws → state kept
    expect(store.getState().sessions["seeed@192.168.1.31"].phase).toBe(
      "running",
    );
  });
});

describe("sessionsStore (identity-v3: deviceId keys)", () => {
  it("register keys the session by username@deviceId", () => {
    const gw = fakeGateway();
    const store = createSessionsStore(gw);
    store.getState().register(
      {
        host: "192.168.2.18",
        username: "seeed",
        password: "pw",
        deviceId: "5dbfb124",
        displayName: "robotics",
      },
      { host: "192.168.2.18", deviceId: "5dbfb124" },
    );

    const s = store.getState();
    expect(s.order).toEqual(["seeed@5dbfb124"]);
    expect(s.sessions["seeed@5dbfb124"].displayName).toBe("robotics");
    expect(s.sessions["seeed@5dbfb124"].deviceId).toBe("5dbfb124");
  });

  it("the same device via its other address does NOT create a second tab", () => {
    const gw = fakeGateway();
    const store = createSessionsStore(gw);
    store.getState().register(
      {
        host: "192.168.2.18",
        username: "seeed",
        password: "pw",
        deviceId: "5dbfb124",
        displayName: "robotics",
      },
      null,
    );
    // Same machine-id, entered through the Tailscale address this time.
    store.getState().register(
      {
        host: "100.114.170.49",
        username: "seeed",
        password: "pw",
        deviceId: "5dbfb124",
        displayName: "robotics",
      },
      null,
    );

    const s = store.getState();
    expect(s.order).toEqual(["seeed@5dbfb124"]); // ONE tab
    expect(s.sessions["seeed@5dbfb124"].host).toBe("100.114.170.49");
  });

  it("two different devices are two tabs with distinct keys", () => {
    const store = createSessionsStore(fakeGateway());
    store.getState().register(
      { host: "192.168.2.18", username: "seeed", password: "pw", deviceId: "id-a", displayName: "robotics" },
      null,
    );
    store.getState().register(
      { host: "192.168.100.164", username: "seeed", password: "pw", deviceId: "id-b", displayName: "mini" },
      null,
    );
    expect(store.getState().order).toEqual(["seeed@id-a", "seeed@id-b"]);
  });

  it("relaunch forwards the deviceId to the desktop gateway", async () => {
    const gw = fakeGateway();
    const store = createSessionsStore(gw);
    store.getState().register(
      { host: "192.168.2.18", username: "seeed", password: "pw", deviceId: "5dbfb124", displayName: "robotics" },
      null,
    );
    gw.statuses.set("seeed@5dbfb124", { kind: "notRunning" });
    await store.getState().pollStatuses();

    store.getState().focusTab("seeed@5dbfb124");
    await flush();
    expect(gw.calls).toContain("launch:seeed@5dbfb124:true");
  });

  it("sessionPathLabel shows the current path kind + address", () => {
    const store = createSessionsStore(fakeGateway());
    store.getState().register(
      { host: "100.114.170.49", username: "seeed", password: "pw", deviceId: "5dbfb124", displayName: "robotics" },
      null,
    );
    const session = store.getState().sessions["seeed@5dbfb124"];
    expect(sessionPathLabel(session)).toBe("Tailscale 100.114.170.49");

    store.getState().register(
      { host: "192.168.2.18", username: "seeed", password: "pw", deviceId: "id-b", displayName: "mini" },
      null,
    );
    expect(sessionPathLabel(store.getState().sessions["seeed@id-b"])).toBe(
      "LAN 192.168.2.18",
    );
  });
});
