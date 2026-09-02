import { useEffect, useState } from "react";
import { useConnectionStore } from "../stores/connectionStore";
import { useSessionsStore } from "../stores/sessionsStore";
import { AppInfo, getAppInfo } from "../lib/ipc";
import { TauriSessionService } from "../features/connection/tauriService";
import { HomeScreen } from "../features/connection/components/HomeScreen";
import { ProvisioningScreen } from "../features/connection/components/ProvisioningScreen";
import { ReadyScreen } from "../features/connection/components/ReadyScreen";
import { DesktopRunningScreen } from "../features/connection/components/DesktopRunningScreen";
import { ErrorScreen } from "../features/connection/components/ErrorScreen";
import { HostKeyPromptScreen } from "../features/connection/components/HostKeyPromptScreen";
import { DevScenarioPicker } from "../features/connection/components/DevScenarioPicker";
import { DevNetworkProbe } from "../features/connection/components/DevNetworkProbe";
import { UpdateChecker } from "../features/update/UpdateChecker";
import { SessionTabBar } from "../components/SessionTabBar";

const sessionIpc = new TauriSessionService();

function App() {
  const state = useConnectionStore((s) => s.state);
  const device = useConnectionStore((s) => s.device);
  const mode = useConnectionStore((s) => s.mode);
  const hasSessions = useSessionsStore((s) => s.order.length > 0);
  const [appInfo, setAppInfo] = useState<AppInfo | null>(null);

  useEffect(() => {
    void getAppInfo().then(setAppInfo);
    // V0.3: restore the remembered device and auto-reconnect when a stored
    // password exists. Guarded once-per-run inside the store (StrictMode).
    void useConnectionStore.getState().initRemembered();
  }, []);

  /** Wizard states render webview screens, so native desktops must hide. */
  const wizardActive = state !== "idle" && state !== "desktop_opened";

  // V0.4 handoff: a real-backend desktop launch is adopted by the sessions
  // store (tab bar) and the wizard resets — WITHOUT closing the desktop.
  // Mock/dev mode keeps the legacy single-flow screens (no handoff).
  useEffect(() => {
    if (state !== "desktop_opened" || mode !== "real") return;
    const cs = useConnectionStore.getState();
    useSessionsStore.getState().register(
      {
        host: cs.form.host,
        username: cs.form.username,
        password: cs.form.password,
      },
      cs.device,
    );
    cs.handoff();
  }, [state, mode]);

  // Native-view convergence: the backend's focused view must always match the
  // UI (wizard visible → hide all; idle → focus the active tab, if any).
  useEffect(() => {
    if (wizardActive) {
      void sessionIpc.focus(null);
      return;
    }
    if (state === "idle") {
      const ss = useSessionsStore.getState();
      const active = ss.activeId ? ss.sessions[ss.activeId] : null;
      if (active && active.phase === "running") {
        void sessionIpc.focus(active.id);
      } else {
        void sessionIpc.focus(null);
      }
    }
  }, [state, wizardActive]);

  // Session exit detection for every connected device (1s, legacy parity).
  useEffect(() => {
    if (!hasSessions) return;
    const id = setInterval(() => {
      if (useConnectionStore.getState().mode === "real") {
        void useSessionsStore.getState().pollStatuses();
      }
    }, 1000);
    return () => clearInterval(id);
  }, [hasSessions]);

  let screen;
  if (state === "idle") {
    screen = <HomeScreen />;
  } else if (state === "error") {
    screen = <ErrorScreen />;
  } else if (state === "host_key_unknown" || state === "host_key_changed") {
    screen = <HostKeyPromptScreen />;
  } else if (state === "ready") {
    screen = <ReadyScreen device={device} />;
  } else if (state === "desktop_opened") {
    screen = <DesktopRunningScreen />;
  } else {
    // connecting_ssh / authenticating / detecting_device /
    // checking_environment / provisioning / verifying / provision_required /
    // launching_rdp
    screen = <ProvisioningScreen />;
  }

  return (
    <div className="jr-bg flex h-full flex-col text-slate-900 dark:text-slate-100">
      {/* V0.4: the tab bar lives in the 44px strip the native desktop views
          leave free at the top — it stays clickable while a desktop is on
          screen (that is the quick-switch surface). */}
      {hasSessions && <SessionTabBar />}
      {/* Keyed by state so every screen transition plays the fade-up entrance
          animation; store actions and polling effects are unaffected. */}
      <main key={state} className="jr-enter flex-1 overflow-y-auto p-6">
        {screen}
      </main>
      <footer className="flex items-center justify-between border-t border-slate-200/80 px-6 py-2 dark:border-slate-800">
        <div className="flex items-center gap-4">
          {import.meta.env.DEV && <DevScenarioPicker />}
          {import.meta.env.DEV && <DevNetworkProbe />}
        </div>
        <UpdateChecker currentVersion={appInfo?.version ?? "?"} />
      </footer>
    </div>
  );
}

export default App;
