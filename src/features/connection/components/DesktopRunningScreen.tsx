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
        className="flex h-14 w-14 items-center justify-center rounded-full bg-sky-500/10 text-sky-600 dark:bg-sky-500/15 dark:text-sky-400"
        aria-hidden
      >
        <svg width="26" height="26" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.8" strokeLinecap="round" strokeLinejoin="round">
          <rect x="2" y="3" width="20" height="14" rx="2" />
          <path d="M8 21h8M12 17v4" />
        </svg>
      </span>
      <h2 className="mt-4 flex items-center gap-2.5 text-lg font-semibold text-slate-900 dark:text-slate-100">
        Desktop open
        <span className="jr-dot-live inline-block h-2 w-2 rounded-full bg-emerald-500" aria-hidden />
      </h2>
      <p className="mt-2 text-sm text-slate-500 dark:text-slate-400">
        The Jetson desktop is showing in its own window.
      </p>
      {device && (
        <div className="mt-4 w-full text-left">
          <DeviceCard device={device} />
        </div>
      )}
      <div className="mt-7 flex gap-3">
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
