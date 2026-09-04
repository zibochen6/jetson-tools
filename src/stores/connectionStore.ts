import { create } from "zustand";
import {
  ConnectionError,
  ConnectionFailure,
  ConnectionInput,
  ConnectionProgress,
  ConnectionState,
  describeError,
  HostKeyDecision,
  HostKeyInfo,
  JetsonDevice,
  PrepareResult,
  ProvisionStage,
  RemoteEnvironmentReport,
  RdpStatus,
} from "../features/connection/types";
import {
  ConnectionService,
  isAbortError,
  MockConnectionService,
  MockScenario,
} from "../features/connection/service";
import { TauriConnectionService, probeDevicePaths } from "../features/connection/tauriService";
import {
  ConnectionForm,
  isValidDisplayName,
  isFormValid,
} from "../features/connection/validation";
import {
  DeviceMemoryGateway,
  SavedDeviceInfo,
  TauriDeviceMemoryGateway,
} from "../features/connection/savedDevice";
import { candidateAddresses, deviceKey } from "../features/connection/paths";
import { useSessionsStore } from "./sessionsStore";

export interface ConnectionStore {
  // snapshot state
  state: ConnectionState;
  progress: ConnectionProgress | null;
  device: JetsonDevice | null;
  environment: RemoteEnvironmentReport | null;
  error: ConnectionError | null;
  hostKey: HostKeyInfo | null;
  previousKey: HostKeyInfo | null;
  /** True once bootstrap has actually started (Cancel is deliberately ignored). */
  provisioningLocked: boolean;
  /** Active desktop process status (drives exit detection). */
  rdpStatus: RdpStatus | null;
  /** Which phase failed last — drives whether `retry` reconnects or relaunches. */
  lastFailure: "connect" | "launch" | null;
  /** Dev-only; drives mock outcomes. Not part of the real product. */
  scenario: MockScenario;
  /** Dev/prod toggle over the connection backend. */
  mode: "real" | "mock";

  /**
   * Remembered devices (identity-v3), most recently connected first.
   * `hasPassword` signals whether the backend holds a stored password for
   * that device — when true the form may leave the password blank and the
   * backend resolves it itself. Never contains the actual secret.
   */
  savedDevices: SavedDeviceInfo[];
  /** The overview's add-device form is open (form cleared for a NEW device). */
  addingDevice: boolean;
  /**
   * Transient notice shown on the overview (e.g. "已作为「X」连接" when the
   * user entered another address of an already-connected device).
   */
  notice: string | null;

  // transient form (in-memory only; password is NEVER persisted)
  form: ConnectionForm;

  // actions
  setForm: (partial: Partial<ConnectionForm>) => void;
  setScenario: (scenario: MockScenario) => void;
  setMode: (mode: "real" | "mock") => void;
  /** Load persisted device memory; auto-connects the most recent device. */
  initRemembered: () => Promise<void>;
  /** Forget ONE remembered device (JSON entry + secret-store password). */
  forgetDevice: (device: SavedDeviceInfo) => Promise<void>;
  /** Open the overview's add-device form with a CLEARED form. */
  openAddDevice: () => void;
  /** Collapse the add-device form. */
  closeAddDevice: () => void;
  /** Clear the transient overview notice. */
  clearNotice: () => void;
  connect: () => Promise<void>;
  /**
   * Naming gate (identity-v3): the mandatory display name for a brand-new
   * machine-id. Returns false when the name is invalid (screen shows inline
   * error); a valid name continues the flow (prepare → launch).
   */
  confirmDeviceName: (name: string) => Promise<boolean>;
  trustKey: () => void;
  replaceKey: () => void;
  cancel: () => void;
  retry: () => void;
  back: () => void;
  launchDesktop: () => Promise<void>;
  closeDesktop: () => Promise<void>;
  disconnect: () => Promise<void>;
  refreshRdpStatus: () => Promise<void>;
  /**
   * Multi-device (V0.4): the just-opened desktop was registered with the
   * sessions store — reset the wizard to idle WITHOUT closing the desktop.
   * Only used by the app shell (real backend); injected-store tests never
   * call it.
   */
  handoff: () => void;
}

/**
 * Same physical device for local list updates: same user AND (same machine-id
 * OR, for legacy entries without one, a shared address).
 */
export function sameSavedDevice(
  a: { username: string; deviceId?: string | null; paths?: { address: string }[]; lastUsedPath?: string | null },
  b: { username: string; deviceId?: string | null; paths?: { address: string }[]; lastUsedPath?: string | null },
): boolean {
  if (a.username !== b.username) return false;
  if (a.deviceId && b.deviceId) return a.deviceId === b.deviceId;
  const addrsA = new Set([
    a.lastUsedPath,
    ...(a.paths ?? []).map((p) => p.address),
  ]);
  return (
    Boolean(b.lastUsedPath && addrsA.has(b.lastUsedPath)) ||
    (b.paths ?? []).some((p) => addrsA.has(p.address))
  );
}

export const initialForm: ConnectionForm = {
  host: "",
  username: "",
  password: "",
  remember: false,
  deviceId: null,
};

/**
 * The remembered device matching the form (if any) — only entries WITH a
 * stored password can exempt the form from typing one. Identity-aware (v3):
 * matches by deviceId, or when the typed address is one of the device's known
 * paths (both addresses of one Jetson are one device).
 */
export function matchSavedDevice(
  devices: SavedDeviceInfo[],
  host: string,
  username: string,
  deviceId?: string | null,
): SavedDeviceInfo | null {
  const h = host.trim();
  const u = username.trim();
  return (
    devices.find((d) => {
      if (!d.hasPassword || d.username !== u) return false;
      if (deviceId && d.deviceId && d.deviceId === deviceId) return true;
      const addrs = [
        d.lastUsedPath,
        ...(d.paths ?? []).map((p) => p.address),
      ].filter((v): v is string => Boolean(v));
      return addrs.includes(h);
    }) ?? null
  );
}

/** The remembered v3 device for a machine-id, if any (regardless of secret). */
export function findSavedByDeviceId(
  devices: SavedDeviceInfo[],
  deviceId: string | null | undefined,
  username: string,
): SavedDeviceInfo | null {
  if (!deviceId) return null;
  return (
    devices.find((d) => d.deviceId === deviceId && d.username === username) ??
    null
  );
}

/** Map a provision stage to the ConnectionState shown while it runs. */
export function stageToState(stage: ProvisionStage): ConnectionState | null {
  switch (stage) {
    case "checking_environment":
      return "checking_environment";
    case "provision_required":
      return "provision_required";
    case "preflight":
    case "uploading":
    case "installing_packages":
    case "configuring_session":
    case "starting_service":
      return "provisioning";
    case "verifying":
      return "verifying";
    default:
      return null; // already_ready / complete → final result handles
  }
}

const RUNNING_STAGES: ProvisionStage[] = [
  "installing_packages",
  "configuring_session",
  "starting_service",
];

/**
 * Connect failures that mean "this ADDRESS didn't work" — try the next
 * candidate path (identity-v3 routing). Everything else surfaces as-is.
 */
const RETRYABLE_CANDIDATE_CODES = new Set([
  "ssh_timeout",
  "auth_failed",
]);

type ServiceFactory = () => ConnectionService;

/**
 * Factory so tests can inject a fake service AND a fake device-memory gateway.
 * The default app instance (`useConnectionStore`) selects a backend from
 * `mode` at connect time and uses the real Tauri device memory.
 */
export function createConnectionStore(
  injectedFactory?: ServiceFactory,
  injectedMemory?: DeviceMemoryGateway,
) {
  // One auto-reconnect attempt per app run — module-level so React
  // StrictMode's double effect in dev can't double-fire it (PRD R9).
  let autoConnectAttempted = false;

  let abort: AbortController | null = null;
  // One service instance per connect flow, reused across prepare→launch→status
  // so a stateful mock (RDP running/exited) stays coherent.
  let currentService: ConnectionService | null = null;
  // The input of the candidate that actually connected (may differ from the
  // form when an alternate path won). Drives prepare/launch/memory sync.
  let connectedInput: ConnectionInput | null = null;
  // The candidate whose host-key prompt is pending — a trust/replace decision
  // must re-dial THAT address, not re-run the whole candidate sweep.
  let pendingDecisionHost: string | null = null;
  // Bounded auto-retry for RDP connection failures (e.g. macOS "Local Network"
  // permission not yet granted on first launch): re-launch a few times so the
  // desktop opens automatically once the user clicks Allow.
  let rdpRetryCount = 0;
  let rdpRetryPending = false;
  const MAX_RDP_RETRIES = 5;

  return create<ConnectionStore>()((set, get) => {
    const resolveService = (): ConnectionService => {
      if (injectedFactory) return injectedFactory();
      return get().mode === "real"
        ? new TauriConnectionService()
        : new MockConnectionService();
    };

    const resolveMemory = (): DeviceMemoryGateway =>
      injectedMemory ?? new TauriDeviceMemoryGateway();

    /**
     * Candidate addresses for this connect: the typed entry host plus every
     * known path of the matching remembered device. Ordered by parallel TCP
     * probe (lowest RTT first) when the real backend is available; the typed
     * host is always kept, even when unreachable.
     */
    const candidatesFor = async (
      form: ConnectionForm,
      saved: SavedDeviceInfo | null,
      signal: AbortSignal,
    ): Promise<string[]> => {
      const entry = form.host.trim();
      const known = saved ? (saved.paths ?? []).map((p) => p.address) : [];
      const candidates = candidateAddresses(entry, known);
      if (candidates.length <= 1 || signal.aborted) return candidates;

      // Parallel TCP :22 probe → RTT-ordered. Advisory: unreachable entries
      // still stay in the list (at the end) so the typed address is tried.
      if (get().mode !== "real") return candidates;
      const probes = (await probeDevicePaths(candidates)) ?? [];
      if (signal.aborted) return candidates;
      const byAddr = new Map(probes.map((p) => [p.address, p]));
      const reachable = candidates
        .filter((a) => byAddr.get(a)?.reachable)
        .sort(
          (a, b) => (byAddr.get(a)?.rttMs ?? 0) - (byAddr.get(b)?.rttMs ?? 0),
        );
      const unreachable = candidates.filter((a) => !reachable.includes(a));
      const ordered = [...reachable, ...unreachable];
      return ordered.length > 0 ? ordered : candidates;
    };

    const doConnect = async (decision?: HostKeyDecision): Promise<void> => {
      const { form, scenario } = get();
      const saved = matchSavedDevice(
        get().savedDevices,
        form.host,
        form.username,
        form.deviceId,
      );
      if (!isFormValid(form, saved)) return;

      abort?.abort();
      abort = new AbortController();
      const signal = abort.signal;

      set({
        state: "connecting_ssh",
        progress: { state: "connecting_ssh", message: "Connecting to Jetson" },
        device: null,
        environment: null,
        error: null,
        hostKey: null,
        previousKey: null,
        provisioningLocked: false,
        rdpStatus: null,
        lastFailure: null,
        notice: null,
      });

      currentService = resolveService();
      const service = currentService;

      // A trust/replace decision re-dials exactly the address that prompted.
      const candidates = decision && pendingDecisionHost
        ? [pendingDecisionHost]
        : await candidatesFor(form, saved, signal);
      if (signal.aborted) return;

      let lastError: ConnectionFailure | null = null;
      let connected: { device: JetsonDevice; input: ConnectionInput } | null =
        null;

      for (const host of candidates) {
        if (signal.aborted) return;
        const input: ConnectionInput = {
          host,
          username: form.username,
          password: form.password,
          remember: form.remember,
          deviceId: form.deviceId ?? null,
        };
        try {
          const outcome = await service.connect(input, {
            scenario,
            signal,
            hostKeyDecision: decision,
            onProgress: (p) => {
              if (signal.aborted) return;
              set({ state: p.state, progress: p });
            },
          });

          if (signal.aborted) return;

          switch (outcome.kind) {
            case "device":
              connected = { device: outcome.device, input };
              break;
            case "host_key_unknown":
              // The user must decide; remember WHICH address prompted so the
              // decision re-dials it (not the whole candidate sweep).
              pendingDecisionHost = host;
              set({
                state: "host_key_unknown",
                hostKey: outcome.key,
                previousKey: null,
                progress: null,
              });
              return;
            case "host_key_changed":
              pendingDecisionHost = host;
              set({
                state: "host_key_changed",
                hostKey: outcome.current,
                previousKey: outcome.previous,
                progress: null,
              });
              return;
          }
          break; // connected or handled above
        } catch (err) {
          if (signal.aborted || isAbortError(err)) {
            set({
              state: "idle",
              progress: null,
              provisioningLocked: false,
              lastFailure: null,
            });
            return;
          }
          const failure =
            err instanceof ConnectionFailure
              ? err
              : new ConnectionFailure("unknown");
          lastError = failure;
          // "This address didn't work" → try the next path of the same device.
          if (RETRYABLE_CANDIDATE_CODES.has(failure.code)) {
            continue;
          }
          throw failure;
        }
      }

      if (!connected) {
        set({
          state: "error",
          error: lastError
            ? describeError(lastError.code, lastError.detail)
            : describeError("unknown"),
          lastFailure: "connect",
        });
        return;
      }

      const { device } = connected;
      // Enrich with the discovered device id: everything after the probe
      // (prepare, launch, tunnel key, TOFU identity, memory) is keyed by the
      // stable deviceId, not by the entry address.
      const input: ConnectionInput = {
        ...connected.input,
        deviceId: device.deviceId ?? connected.input.deviceId ?? null,
      };
      connectedInput = input;
      pendingDecisionHost = null;
      set({ device });

      // A session for this machine-id is already live (the user entered
      // another address of the same board): reuse it, never open a second
      // desktop for one device.
      const key = deviceKey(input.username, device.deviceId, device.host);
      const sessions = useSessionsStore.getState();
      const existing = sessions.sessions[key];
      if (existing) {
        sessions.focusTab(key);
        set({
          state: "idle",
          progress: null,
          notice: existing.displayName
            ? `已作为「${existing.displayName}」连接`
            : "该设备已在连接中",
          device: null,
          error: null,
        });
        // Keep the device memory fresh (paths changed?) without reopening.
        syncMemoryAfterConnect(input, device, existing.displayName ?? null);
        return;
      }

      // Mandatory naming gate for a brand-new device id (identity-v3): the
      // display name is required BEFORE provision / desktop.
      const savedNow = findSavedByDeviceId(
        get().savedDevices,
        device.deviceId,
        input.username,
      );
      const needsName =
        !!device.deviceId && !(savedNow?.displayName ?? "").trim().length;
      if (needsName) {
        set({
          state: "naming_device",
          progress: null,
        });
        return;
      }

      // Auth verified — record/forget per the remember checkbox (paths are
      // refreshed from what the device just reported).
      syncMemoryAfterConnect(input, device, savedNow?.displayName ?? null);

      await prepareAndLaunch(service, input, scenario, signal);
    };

    /**
     * Prepare (check → provision → verify) then auto-launch the desktop,
     * mapping failures to the error screen. Shared by the normal connect flow
     * and the post-naming continuation.
     */
    const prepareAndLaunch = async (
      service: ConnectionService,
      input: ConnectionInput,
      scenario: MockScenario,
      signal: AbortSignal,
    ): Promise<void> => {
      set({
        state: "checking_environment",
        progress: {
          state: "checking_environment",
          message: "Checking remote desktop",
        },
      });
      try {
        await prepareDevice(service, input, scenario, signal);
      } catch (err) {
        if (signal.aborted || isAbortError(err)) {
          set({
            state: "idle",
            progress: null,
            provisioningLocked: false,
            lastFailure: null,
          });
        } else if (err instanceof ConnectionFailure) {
          set({
            state: "error",
            error: describeError(err.code, err.detail),
            lastFailure: "connect",
          });
        } else {
          set({
            state: "error",
            error: describeError("unknown"),
            lastFailure: "connect",
          });
        }
        return;
      }
      if (signal.aborted) return;
      // Desktop now ready → open it (auto-launch, PRD §52).
      if (get().state === "ready") {
        await get().launchDesktop();
      }
    };

    /**
     * Sync the device memory after a successful device probe (auth verified).
     * Best-effort by design (PRD R8): a memory I/O failure never breaks the
     * connection flow, it only affects the next launch's auto-reconnect.
     * Identity-v3: keyed by machine-id; the device's CURRENT path list
     * overwrites the stored one (stale addresses dropped); legacy v2
     * duplicates of the same board are merged by the backend.
     */
    const syncMemoryAfterConnect = (
      input: ConnectionInput,
      device: JetsonDevice,
      displayName: string | null,
    ): void => {
      const memory = resolveMemory();
      const paths = device.paths ?? [];
      const entry: SavedDeviceInfo = {
        deviceId: device.deviceId ?? null,
        username: input.username,
        displayName,
        paths,
        lastUsedPath: input.host,
        hasPassword: true,
      };

      const upsertLocal = () =>
        set((s) => ({
          savedDevices: [
            entry,
            ...s.savedDevices.filter((d) => !sameSavedDevice(d, entry)),
          ],
        }));

      if (input.remember) {
        // Optimistic local upsert FIRST: the session handoff reads the
        // display name from savedDevices synchronously right after launch.
        // A failing disk write only affects the next app run (best-effort).
        upsertLocal();
        void (async () => {
          try {
            await memory.save({
              deviceId: entry.deviceId,
              username: entry.username,
              displayName: entry.displayName,
              paths: entry.paths,
              entryHost: input.host,
              // Blank password = the memory already holds the secret; the
              // backend keeps it and only refreshes identity + paths.
              password: input.password,
            });
          } catch {
            // best-effort: connection already succeeded
          }
        })();
        return;
      }

      // remember unchecked: connecting to a remembered device means the user
      // wants THAT one forgotten (PRD R6). Other devices are untouched.
      const remembered =
        findSavedByDeviceId(get().savedDevices, device.deviceId, input.username) ??
        matchSavedDevice(get().savedDevices, input.host, input.username);
      if (remembered) {
        void (async () => {
          try {
            await memory.forget({
              deviceId: remembered.deviceId,
              host:
                (remembered.paths ?? []).find((p) => p.address === input.host)
                  ?.address ?? remembered.lastUsedPath,
              username: remembered.username,
            });
            set((s) => ({
              savedDevices: s.savedDevices.filter((d) => d !== remembered),
            }));
          } catch {
            // best-effort
          }
        })();
      }
    };

    const prepareDevice = async (
      service: ConnectionService,
      input: ConnectionInput,
      scenario: MockScenario,
      signal: AbortSignal,
    ): Promise<void> => {
      const prepared: PrepareResult = await service.prepare(input, {
        scenario,
        signal,
        onEvent: (event) => {
          if (signal.aborted) return;
          if (RUNNING_STAGES.includes(event.stage)) {
            set({ provisioningLocked: true });
          }
          const next = stageToState(event.stage);
          if (next) {
            set({
              state: next,
              progress: { state: next, message: event.message },
            });
          }
        },
      });

      if (signal.aborted) return;

      switch (prepared.kind) {
        case "ready":
          set({
            state: "ready",
            environment: prepared.environment,
            provisioningLocked: false,
            progress: { state: "ready", message: "Remote desktop is ready" },
          });
          break;
        case "hostKeyUnknown":
          set({
            state: "host_key_unknown",
            hostKey: prepared.key,
            previousKey: null,
            progress: null,
          });
          break;
        case "hostKeyChanged":
          set({
            state: "host_key_changed",
            hostKey: prepared.current,
            previousKey: prepared.previous,
            progress: null,
          });
          break;
      }
    };

    /** Continue the flow after the naming gate accepted a display name. */
    const continueAfterNaming = async (displayName: string): Promise<void> => {
      const { device, scenario } = get();
      const input = connectedInput;
      if (!device || !input) return;
      const service = currentService ?? (currentService = resolveService());
      const signal = abort?.signal ?? new AbortController().signal;

      // Persist identity + name (+ password when typed) before provisioning.
      syncMemoryAfterConnect(input, device, displayName);

      await prepareAndLaunch(service, input, scenario, signal);
    };

    const launchDesktop = async (): Promise<void> => {
      const { form, device } = get();
      const saved = matchSavedDevice(
        get().savedDevices,
        form.host,
        form.username,
        form.deviceId,
      );
      if (!device || !isFormValid(form, saved)) return;

      rdpRetryCount = 0;
      rdpRetryPending = false;
      const service = currentService ?? (currentService = resolveService());
      set({
        state: "launching_rdp",
        progress: { state: "launching_rdp", message: "Opening desktop" },
      });
      try {
        // Launch against the address that actually connected (an alternate
        // path may have won the RTT race), keyed by the stable device id.
        const input = connectedInput ?? {
          host: form.host,
          username: form.username,
          password: form.password,
          remember: form.remember,
          deviceId: form.deviceId ?? null,
        };
        await service.launch(
          {
            host: input.host,
            username: input.username,
            password: input.password,
            deviceId: device.deviceId ?? input.deviceId ?? null,
          },
          // Keyed session (identity-v3): one desktop per DEVICE, quick
          // switchable via the tab bar without reconnecting. Both addresses
          // of one Jetson map to the same key.
          {
            scenario: get().scenario,
            sessionId: deviceKey(
              input.username,
              device.deviceId ?? input.deviceId,
              input.host,
            ),
          },
        );
        set({
          state: "desktop_opened",
          rdpStatus: { kind: "running" },
          progress: null,
        });
      } catch (err) {
        set({
          state: "error",
          error:
            err instanceof ConnectionFailure
              ? describeError(err.code, err.detail)
              : describeError("rdp_failed"),
          lastFailure: "launch",
          rdpStatus: null,
        });
      }
    };

    const closeDesktop = async (): Promise<void> => {
      const service = currentService ?? resolveService();
      try {
        await service.close();
      } catch {
        // best-effort; the remote Xorg/XFCE session is left alive either way
      }
      set({
        state: "ready",
        rdpStatus: null,
        progress: { state: "ready", message: "Remote desktop is ready" },
      });
    };

    const disconnect = async (): Promise<void> => {
      const service = currentService ?? resolveService();
      try {
        await service.close();
      } catch {
        // best-effort
      }
      abort?.abort();
      set({
        state: "idle",
        progress: null,
        device: null,
        environment: null,
        error: null,
        hostKey: null,
        previousKey: null,
        provisioningLocked: false,
        rdpStatus: null,
        lastFailure: null,
      });
    };

    const refreshRdpStatus = async (): Promise<void> => {
      if (get().state !== "desktop_opened") return;
      const service = currentService ?? resolveService();
      try {
        const status = await service.status();
        if (status.kind === "running") {
          rdpRetryCount = 0;
          rdpRetryPending = false;
          set({ rdpStatus: status });
        } else if (status.kind === "exited" && status.error) {
          // The embedded bridge surfaced a connection failure (e.g. macOS
          // "Local Network" permission not yet granted → connect fails fast).
          // Auto-retry a bounded number of times; when the user clicks Allow the
          // next launch succeeds, otherwise surface it as an error.
          if (rdpRetryCount >= MAX_RDP_RETRIES) {
            rdpRetryCount = 0;
            rdpRetryPending = false;
            set({
              state: "error",
              error: describeError("rdp_connection_failed"),
              lastFailure: "launch",
              rdpStatus: null,
            });
          } else if (!rdpRetryPending) {
            rdpRetryPending = true;
            rdpRetryCount += 1;
            setTimeout(() => {
              rdpRetryPending = false;
              if (get().state === "desktop_opened") void get().launchDesktop();
            }, 2000);
          }
        } else {
          // clean close (notRunning / exited without error) — desktop closed.
          rdpRetryCount = 0;
          rdpRetryPending = false;
          set({
            state: "ready",
            rdpStatus: null,
            progress: { state: "ready", message: "Remote desktop is ready" },
          });
        }
      } catch {
        // transient status probe failure — keep current state
      }
    };

    return {
      state: "idle",
      progress: null,
      device: null,
      environment: null,
      error: null,
      hostKey: null,
      previousKey: null,
      provisioningLocked: false,
      rdpStatus: null,
      lastFailure: null,
      scenario: "success",
      mode: "real",
      savedDevices: [],
      addingDevice: false,
      notice: null,
      form: initialForm,

      setForm: (partial) => set((s) => ({ form: { ...s.form, ...partial } })),
      setScenario: (scenario) => set({ scenario }),
      setMode: (mode) => set({ mode }),

      initRemembered: async () => {
        // Once per app run; StrictMode's double effect can't double-fire.
        if (autoConnectAttempted) return;
        autoConnectAttempted = true;
        if (get().state !== "idle") return;

        const saved = await resolveMemory().loadAll().catch(() => []);
        if (saved.length === 0) return;
        set({ savedDevices: saved });

        // Only the most recent device may auto-connect; launching every
        // remembered desktop at once would pop N windows. If its secret is
        // gone, still prefill that device so the user can replace it.
        const mru = saved[0];
        const entry =
          mru.lastUsedPath ?? (mru.paths ?? [])[0]?.address ?? "";
        set({
          form: {
            host: entry,
            username: mru.username,
            password: "",
            remember: true,
            deviceId: mru.deviceId ?? null,
          },
        });
        if (mru.hasPassword) void get().connect();
      },

      forgetDevice: async (device) => {
        try {
          await resolveMemory().forget({
            deviceId: device.deviceId,
            host: device.lastUsedPath ?? (device.paths ?? [])[0]?.address ?? null,
            username: device.username,
          });
        } catch {
          // best-effort
        }
        set((s) => ({
          savedDevices: s.savedDevices.filter((d) => !sameSavedDevice(d, device)),
        }));
      },

      openAddDevice: () => {
        // A CLEARED form: adding a device must never prefill the credentials
        // of an already-connected one (V0.4 fix).
        set({
          addingDevice: true,
          form: { ...initialForm, remember: true },
        });
      },
      closeAddDevice: () => set({ addingDevice: false }),
      clearNotice: () => set({ notice: null }),

      connect: () => doConnect(undefined),
      confirmDeviceName: async (name) => {
        if (!isValidDisplayName(name)) return false;
        await continueAfterNaming(name.trim());
        return true;
      },
      trustKey: () => {
        const key = get().hostKey;
        if (key) void doConnect({ action: "trustAndConnect", key });
      },
      replaceKey: () => {
        const key = get().hostKey;
        if (key) void doConnect({ action: "replaceAndConnect", key });
      },

      cancel: () => {
        if (get().provisioningLocked) return; // destructive phase: refuse
        abort?.abort();
        set({ state: "idle", progress: null, rdpStatus: null });
      },
      retry: () => {
        if (get().lastFailure === "launch") {
          void get().launchDesktop();
        } else {
          void doConnect(undefined);
        }
      },
      back: () => {
        abort?.abort();
        connectedInput = null;
        pendingDecisionHost = null;
        set({
          state: "idle",
          progress: null,
          device: null,
          environment: null,
          error: null,
          hostKey: null,
          previousKey: null,
          provisioningLocked: false,
          rdpStatus: null,
          lastFailure: null,
        });
      },
      launchDesktop: () => launchDesktop(),
      closeDesktop: () => closeDesktop(),
      disconnect: () => disconnect(),
      handoff: () => {
        // The desktop stays alive under the sessions store's key; only the
        // wizard resets (no service.close() — that would kill the desktop).
        abort?.abort();
        connectedInput = null;
        set({
          state: "idle",
          progress: null,
          device: null,
          environment: null,
          error: null,
          hostKey: null,
          previousKey: null,
          provisioningLocked: false,
          rdpStatus: null,
          lastFailure: null,
        });
      },
      refreshRdpStatus: () => refreshRdpStatus(),
    };
  });
}

/** The single app-wide store instance. */
export const useConnectionStore = createConnectionStore();
