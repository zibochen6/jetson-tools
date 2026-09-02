import { useEffect, useState } from "react";
import { useConnectionStore } from "../stores/connectionStore";
import { AppInfo, getAppInfo } from "../lib/ipc";
import { HomeScreen } from "../features/connection/components/HomeScreen";
import { ProvisioningScreen } from "../features/connection/components/ProvisioningScreen";
import { ReadyScreen } from "../features/connection/components/ReadyScreen";
import { DesktopRunningScreen } from "../features/connection/components/DesktopRunningScreen";
import { ErrorScreen } from "../features/connection/components/ErrorScreen";
import { HostKeyPromptScreen } from "../features/connection/components/HostKeyPromptScreen";
import { DevScenarioPicker } from "../features/connection/components/DevScenarioPicker";
import { DevNetworkProbe } from "../features/connection/components/DevNetworkProbe";
import { UpdateChecker } from "../features/update/UpdateChecker";
import { devTunnelLabel } from "../features/connection/tauriService";

function App() {
  const state = useConnectionStore((s) => s.state);
  const device = useConnectionStore((s) => s.device);
  const [appInfo, setAppInfo] = useState<AppInfo | null>(null);

  useEffect(() => {
    void getAppInfo().then(setAppInfo);
    // V0.3: restore the remembered device and auto-reconnect when a stored
    // password exists. Guarded once-per-run inside the store (StrictMode).
    void useConnectionStore.getState().initRemembered();
  }, []);

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
    <div className="flex h-full flex-col bg-[#f5f5f7] text-zinc-900 dark:bg-zinc-900 dark:text-zinc-100">
      <main className="flex-1 overflow-y-auto p-6">{screen}</main>
      <footer className="flex items-center justify-between border-t border-zinc-200 px-6 py-2 dark:border-zinc-700">
        <div className="flex items-center gap-4">
          {import.meta.env.DEV && <DevScenarioPicker />}
          {import.meta.env.DEV && <DevNetworkProbe />}
          {devTunnelLabel && (
            <span
              className="rounded border border-zinc-300 px-1 py-0.5 text-xs text-zinc-500 dark:border-zinc-600 dark:text-zinc-400"
              title="Tunnel mode: SSH + RDP ride the loopback ssh tunnel (KI-004)"
            >
              TUNNEL {devTunnelLabel}
            </span>
          )}
        </div>
        <UpdateChecker currentVersion={appInfo?.version ?? "?"} />
      </footer>
    </div>
  );
}

export default App;