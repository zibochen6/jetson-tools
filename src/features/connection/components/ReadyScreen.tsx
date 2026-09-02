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
      <h2 className="mt-3 text-lg font-semibold text-zinc-900 dark:text-zinc-100">
        Jetson ready
      </h2>
      {device && (
        <div className="mt-4 w-full text-left">
          <DeviceCard device={device} />
        </div>
      )}
      {environment && (
        <p className="mt-3 text-sm text-zinc-500 dark:text-zinc-400">
          Remote desktop is ready.
        </p>
      )}
      <div className="mt-6 flex gap-3">
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
      className="flex h-12 w-12 items-center justify-center rounded-full bg-emerald-100 text-2xl text-emerald-600 dark:bg-emerald-900/40 dark:text-emerald-400"
      aria-hidden
    >
      ✓
    </span>
  );
}