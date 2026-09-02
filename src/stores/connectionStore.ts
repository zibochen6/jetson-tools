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
import { TauriConnectionService } from "../features/connection/tauriService";
import { ConnectionForm, isFormValid } from "../features/connection/validation";
import {
  DeviceMemoryGateway,
  SavedDeviceInfo,
  TauriDeviceMemoryGateway,
} from "../features/connection/savedDevice";

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
   * The last remembered device (V0.3). `hasPassword` signals whether the
   * backend holds a stored password — when true the form may leave the
   * password blank and the backend resolves it itself. Never contains the
   * actual secret.
   */
  savedDevice: SavedDeviceInfo | null;

  // transient form (in-memory only; password is NEVER persisted)
  form: ConnectionForm;

  // actions
  setForm: (partial: Partial<ConnectionForm>) => void;
  setScenario: (scenario: MockScenario) => void;
  setMode: (mode: "real" | "mock") => void;
  /** Load persisted device memory; auto-connects when a password exists. */
  initRemembered: () => Promise<void>;
  /** Forget the remembered device (JSON + OS keychain entry). */
  forgetDevice: () => Promise<void>;
  connect: () => Promise<void>;
  trustKey: () => void;
  replaceKey: () => void;
  cancel: () => void;
  retry: () => void;
  back: () => void;
  launchDesktop: () => Promise<void>;
  closeDesktop: () => Promise<void>;
  disconnect: () => Promise<void>;
  refreshRdpStatus: () => Promise<void>;
}

export const initialForm: ConnectionForm = {
  host: "",
  username: "",
  password: "",
  remember: false,
};

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

    const doConnect = async (decision?: HostKeyDecision): Promise<void> => {
      const { form, scenario, savedDevice } = get();
      if (!isFormValid(form, savedDevice)) return;

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
      });

      currentService = resolveService();
      const service = currentService;
      const input = {
        host: form.host,
        username: form.username,
        password: form.password,
        remember: form.remember,
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
          case "device": {
            const device = outcome.device;
            set({
              device,
              state: "checking_environment",
              progress: {
                state: "checking_environment",
                message: "Checking remote desktop",
              },
            });
            // Auth verified — record/forget per the remember checkbox.
            syncMemoryAfterConnect(input);
            await prepareDevice(service, input, scenario, signal);
            if (signal.aborted) return;
            // Desktop now ready → open it (auto-launch, PRD §52).
            if (get().state === "ready") {
              await get().launchDesktop();
            }
            break;
          }
          case "host_key_unknown":
            set({
              state: "host_key_unknown",
              hostKey: outcome.key,
              previousKey: null,
              progress: null,
            });
            break;
          case "host_key_changed":
            set({
              state: "host_key_changed",
              hostKey: outcome.current,
              previousKey: outcome.previous,
              progress: null,
            });
            break;
        }
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
            error: describeError(err.code),
            lastFailure: "connect",
          });
        } else {
          set({
            state: "error",
            error: describeError("unknown"),
            lastFailure: "connect",
          });
        }
      }
    };

    /**
     * Sync the device memory after a successful device probe (auth verified).
     * Best-effort by design (PRD R8): a memory I/O failure never breaks the
     * connection flow, it only affects the next launch's auto-reconnect.
     */
    const syncMemoryAfterConnect = (input: ConnectionInput): void => {
      const { savedDevice } = get();
      const memory = resolveMemory();

      if (input.remember && input.password !== "") {
        void (async () => {
          try {
            await memory.save({
              host: input.host,
              username: input.username,
              password: input.password,
            });
            set({
              savedDevice: {
                host: input.host,
                username: input.username,
                hasPassword: true,
              },
            });
          } catch {
            // best-effort: connection already succeeded
          }
        })();
        return;
      }

      if (input.remember) {
        // Saved-password connect (blank password): the memory already holds
        // the secret — nothing to rewrite.
        return;
      }

      // remember unchecked: connecting to the SAME remembered device means
      // the user wants it forgotten (PRD R6). Other devices are untouched.
      if (
        savedDevice &&
        savedDevice.host === input.host &&
        savedDevice.username === input.username
      ) {
        void (async () => {
          try {
            await memory.forget();
            set({ savedDevice: null });
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

    const launchDesktop = async (): Promise<void> => {
      const { form, device, savedDevice } = get();
      if (!device || !isFormValid(form, savedDevice)) return;

      rdpRetryCount = 0;
      rdpRetryPending = false;
      const service = currentService ?? (currentService = resolveService());
      set({
        state: "launching_rdp",
        progress: { state: "launching_rdp", message: "Opening desktop" },
      });
      try {
        await service.launch(
          {
            host: form.host,
            username: form.username,
            password: form.password,
          },
          { scenario: get().scenario },
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
              ? describeError(err.code)
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
      savedDevice: null,
      form: initialForm,

      setForm: (partial) => set((s) => ({ form: { ...s.form, ...partial } })),
      setScenario: (scenario) => set({ scenario }),
      setMode: (mode) => set({ mode }),

      initRemembered: async () => {
        // Once per app run; StrictMode's double effect can't double-fire.
        if (autoConnectAttempted) return;
        autoConnectAttempted = true;
        if (get().state !== "idle") return;

        const saved = await resolveMemory().load().catch(() => null);
        if (!saved) return;

        set({
          savedDevice: saved,
          form: {
            host: saved.host,
            username: saved.username,
            password: "",
            remember: true,
          },
        });
        if (saved.hasPassword) {
          void get().connect();
        }
      },

      forgetDevice: async () => {
        try {
          await resolveMemory().forget();
        } catch {
          // best-effort
        }
        set((s) => ({
          savedDevice: null,
          form: { ...s.form, password: "", remember: false },
        }));
      },

      connect: () => doConnect(undefined),
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
      refreshRdpStatus: () => refreshRdpStatus(),
    };
  });
}

/** The single app-wide store instance. */
export const useConnectionStore = createConnectionStore();