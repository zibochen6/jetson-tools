import { JetsonDevice } from "../types";
import { Button } from "../../../components/Button";
import { DeviceCard } from "../../../components/DeviceCard";
import { useConnectionStore } from "../../../stores/connectionStore";

export function ReadyScreen({ device }: { device: JetsonDevice | null }) {
  const launchDesktop = useConnectionStore((s) => s.launchDesktop);
  const disconnect = useConnectionStore((s) => s.disconnect);
  const environment = useConnectionStore((s) => s.environment);

  return (
    <div className="mx-auto flex h-full max-w-sm flex-col items-center justify-center text-center">
      <CheckMark />
      <h2 className="mt-4 text-lg font-semibold text-slate-900 dark:text-slate-100">
        Jetson ready
      </h2>
      {device && (
        <div className="mt-4 w-full text-left">
          <DeviceCard device={device} />
        </div>
      )}
      {environment && (
        <p className="mt-3 text-sm text-slate-500 dark:text-slate-400">
          Remote desktop is ready.
        </p>
      )}
      <div className="mt-7 flex gap-3">
        <Button variant="primary" onClick={() => void launchDesktop()}>
          Open Desktop
        </Button>
        <Button variant="secondary" onClick={() => void disconnect()}>
          Disconnect
        </Button>
      </div>
    </div>
  );
}

function CheckMark() {
  return (
    <span
      className="flex h-14 w-14 items-center justify-center rounded-full bg-emerald-500/10 text-emerald-600 dark:bg-emerald-500/15 dark:text-emerald-400"
      aria-hidden
    >
      <svg width="26" height="26" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2.2" strokeLinecap="round" strokeLinejoin="round">
        <polyline points="20 6 9 17 4 12" />
      </svg>
    </span>
  );
}
