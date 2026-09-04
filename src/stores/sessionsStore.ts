import { create } from "zustand";
import {
  ConnectionFailure,
  JetsonDevice,
  RdpLaunchResult,
} from "../features/connection/types";
import {
  SessionStatusEntry,
  TauriSessionService,
} from "../features/connection/tauriService";
import { deviceKey, pathLabel } from "../features/connection/paths";

/**
 * Multi-device sessions (V0.4). Each entry is a LIVE desktop session keyed by
 * `username@deviceId` (falling back to `username@host` for devices without a
 * machine-id): the RDP connection stays open in the backend while its tab is
 * in the background, so switching never reconnects. Both addresses of one
 * Jetson map to the SAME key — one device, one desktop. The connection wizard
 * (connectionStore) hands a freshly opened desktop over via `register`.
 */

export type SessionPhase =
  | "launching" // (re)launch in flight
  | "running" // desktop on screen OR alive in the background
  | "ready" // session ended cleanly; tab click re-opens the desktop
  | "error"; // relaunch failed; tab click retries

export interface DeviceSession {
  id: string; // `${username}@${deviceId}` — the backend session key
  host: string; // the address that actually connected (entry path)
  username: string;
  /** Stable identity (device-tree serial / machine-id); null for legacy host-keyed devices. */
  deviceId: string | null;
  /** The user-chosen display name; null until the device is named. */
  displayName: string | null;
  /** Transient launch credential; memory-only, never persisted (PRD §29). */
  password: string;
  device: JetsonDevice | null;
  phase: SessionPhase;
  retries: number;
  retryPending: boolean;
}

/** The session's current path shown as small text (LAN / Tailscale + address). */
export function sessionPathLabel(session: DeviceSession): string {
  return pathLabel(session.host);
}

/** Injectable seam for tests (mirrors the connectionStore factory pattern). */
export interface SessionDesktopGateway {
  launch(
    sessionId: string,
    input: {
      host: string;
      username: string;
      password: string;
      deviceId?: string | null;
    },
    options: { focusOnLaunch: boolean },
  ): Promise<RdpLaunchResult>;
  focus(sessionId: string | null): Promise<void>;
  close(sessionId: string): Promise<void>;
  allStatuses(): Promise<SessionStatusEntry[]>;
}

export interface SessionsStore {
  sessions: Record<string, DeviceSession>;
  /** Tab order (insertion order). */
  order: string[];
  /** Focused tab; `null` = device overview (no native view on screen). */
  activeId: string | null;

  /** Wizard handoff: the desktop was just opened — track + focus it. */
  register(input: {
    host: string;
    username: string;
    password: string;
    deviceId?: string | null;
    displayName?: string | null;
  }, device: JetsonDevice | null): void;
  /** Tab click: focus a running desktop, or relaunch a ready/failed one. */
  focusTab(id: string): void;
  /** Show the device overview (hide every native view). */
  showOverview(): void;
  /** Tab ×: close one device's session and drop the tab. */
  closeTab(id: string): void;
  /** 1s poll: exit detection + bounded auto-relaunch for every session. */
  pollStatuses(): Promise<void>;
}

/** Same bounded auto-retry as the legacy single-desktop flow (macOS Local
 * Network permission: fails fast until the user clicks Allow). */
const MAX_RDP_RETRIES = 5;
const RETRY_DELAY_MS = 2000;

export function createSessionsStore(injected?: SessionDesktopGateway) {
  const gateway: SessionDesktopGateway =
    injected ?? new TauriSessionService();

  return create<SessionsStore>()((set, get) => {
    const patch = (id: string, partial: Partial<DeviceSession>) =>
      set((s) => {
        const cur = s.sessions[id];
        if (!cur) return s;
        return { sessions: { ...s.sessions, [id]: { ...cur, ...partial } } };
      });

    /** Re-open a session's desktop (tab click on ready/error, auto-retry).
     * `force` bypasses the running guard — the auto-retry path uses it when
     * the backend session died but the store still shows "running". */
    const relaunch = async (
      id: string,
      force = false,
      focusOnLaunch = true,
    ): Promise<void> => {
      const session = get().sessions[id];
      if (!session) return;
      if (!force && session.phase === "running") return;
      patch(id, { phase: "launching" });
      try {
        await gateway.launch(
          id,
          {
            host: session.host,
            username: session.username,
            password: session.password,
            deviceId: session.deviceId,
          },
          { focusOnLaunch },
        );
        patch(id, { phase: "running", retries: 0, retryPending: false });
        if (focusOnLaunch) {
          set({ activeId: id });
        } else if (get().activeId === id) {
          // Automatic retries always launch hidden to avoid a race where the
          // user switches tabs during reconnect. Re-focus only if this tab is
          // still selected when the new session is ready.
          void gateway.focus(id);
        }
      } catch (err) {
        patch(id, {
          phase: "error",
          retryPending: false,
        });
        if (err instanceof ConnectionFailure && err.code === "saved_password_missing") {
          // Stored secret gone — the overview form is the recovery path.
          set({ activeId: null });
          void gateway.focus(null);
        }
      }
    };

    /** Desktop ended cleanly → keep the tab, hide the (now stale) view. */
    const markExitedClean = (id: string) => {
      patch(id, { phase: "ready", retries: 0, retryPending: false });
      if (get().activeId === id) {
        set({ activeId: null });
        void gateway.focus(null);
      }
    };

    /** Desktop exited WITH an error → bounded auto-relaunch (legacy parity). */
    const handleFailedExit = (id: string) => {
      const session = get().sessions[id];
      if (!session) return;
      if (session.retries >= MAX_RDP_RETRIES) {
        patch(id, { retries: 0, retryPending: false, phase: "error" });
        if (get().activeId === id) {
          set({ activeId: null });
          void gateway.focus(null);
        }
        return;
      }
      if (!session.retryPending) {
        patch(id, { retryPending: true, retries: session.retries + 1 });
        setTimeout(() => {
          if (get().sessions[id]) void relaunch(id, true, false);
        }, RETRY_DELAY_MS);
      }
    };

    return {
      sessions: {},
      order: [],
      activeId: null,

      register: (input, device) => {
        const id = deviceKey(input.username, input.deviceId ?? null, input.host);
        set((s) => {
          const exists = Boolean(s.sessions[id]);
          const session: DeviceSession = {
            id,
            host: input.host,
            username: input.username,
            deviceId: input.deviceId ?? null,
            displayName: input.displayName ?? null,
            password: input.password,
            device,
            phase: "running",
            retries: 0,
            retryPending: false,
          };
          return {
            sessions: { ...s.sessions, [id]: session },
            order: exists ? s.order : [...s.order, id],
            activeId: id,
          };
        });
      },

      focusTab: (id) => {
        const session = get().sessions[id];
        if (!session) return;
        if (session.phase === "running" || session.phase === "launching") {
          set({ activeId: id });
          void gateway.focus(id);
          return;
        }
        void relaunch(id);
      },

      showOverview: () => {
        set({ activeId: null });
        void gateway.focus(null);
      },

      closeTab: (id) => {
        const { sessions, activeId } = get();
        if (!sessions[id]) return;
        set((s) => {
          const next = { ...s.sessions };
          delete next[id];
          return {
            sessions: next,
            order: s.order.filter((x) => x !== id),
            activeId: s.activeId === id ? null : s.activeId,
          };
        });
        void gateway.close(id);
        if (activeId === id) {
          // Keep the UX flowing: surface the next live desktop, if any.
          const nextRunning = get().order.find(
            (x) => get().sessions[x]?.phase === "running",
          );
          if (nextRunning) {
            set({ activeId: nextRunning });
            void gateway.focus(nextRunning);
          }
        }
      },

      pollStatuses: async () => {
        const ids = Object.keys(get().sessions);
        if (ids.length === 0) return;
        let entries: SessionStatusEntry[];
        try {
          entries = await gateway.allStatuses();
        } catch {
          return; // outside Tauri / transient IPC failure — keep last state
        }
        const byId = new Map(entries.map((e) => [e.sessionId, e.status]));
        for (const id of ids) {
          const session = get().sessions[id];
          if (!session) continue;
          const status = byId.get(id);

          if (!status || status.kind === "notRunning") {
            // Backend lost the session (crash/restart) → re-openable.
            if (session.phase === "running" || session.phase === "launching") {
              markExitedClean(id);
            }
            continue;
          }
          if (status.kind === "running") {
            if (
              session.phase !== "running" ||
              session.retries !== 0 ||
              session.retryPending
            ) {
              patch(id, { phase: "running", retries: 0, retryPending: false });
            }
            continue;
          }
          // exited
          if (status.error) {
            handleFailedExit(id);
          } else if (
            session.phase === "running" ||
            session.phase === "launching"
          ) {
            markExitedClean(id);
          }
        }
      },
    };
  });
}

/** The single app-wide sessions store instance. */
export const useSessionsStore = createSessionsStore();
