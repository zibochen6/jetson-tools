import { useEffect } from "react";
import { Button } from "../../../components/Button";
import { DeviceCard } from "../../../components/DeviceCard";
import { useConnectionStore } from "../../../stores/connectionStore";

export function DesktopRunningScreen() {
  const device = useConnectionStore((s) => s.device);
  const closeDesktop = useConnectionStore((s) => s.closeDesktop);
  const disconnect = useConnectionStore((s) => s.disconnect);
  const refreshRdpStatus = useConnectionStore((s) => s.refreshRdpStatus);

  // Poll the sidecar so we return to "ready" when the window is closed.
  useEffect(() => {
    const id = setInterval(() => {
      void refreshRdpStatus();
    }, 1000);
    return () => clearInterval(id);
  }, [refreshRdpStatus]);

  return (
    <div className="mx-auto flex h-full max-w-sm flex-col items-center justify-center text-center">
      <span
        className="flex h-12 w-12 items-center justify-center rounded-full bg-blue-100 text-2xl text-blue-600 dark:bg-blue-900/40 dark:text-blue-400"
        aria-hidden
      >
        ▣
      </span>
      <h2 className="mt-3 text-lg font-semibold text-zinc-900 dark:text-zinc-100">
        Desktop open
      </h2>
      <p className="mt-2 text-sm text-zinc-500 dark:text-zinc-400">
        The Jetson desktop is showing in its own window.
      </p>
      {device && (
        <div className="mt-4 w-full text-left">
          <DeviceCard device={device} />
        </div>
      )}
      <div className="mt-6 flex gap-3">
        <Button variant="secondary" onClick={() => void closeDesktop()}>
          Close Desktop
        </Button>
        <Button variant="secondary" onClick={() => void disconnect()}>
          Disconnect
        </Button>
      </div>
    </div>
  );
}