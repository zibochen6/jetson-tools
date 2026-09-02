import { Button } from "../../../components/Button";
import { Spinner } from "../../../components/Spinner";
import { useConnectionStore } from "../../../stores/connectionStore";

export function ProvisioningScreen() {
  const progress = useConnectionStore((s) => s.progress);
  const state = useConnectionStore((s) => s.state);
  const cancel = useConnectionStore((s) => s.cancel);
  const provisioningLocked = useConnectionStore((s) => s.provisioningLocked);

  const title = state === "launching_rdp" ? "Opening desktop" : "Preparing your Jetson";

  return (
    <div className="flex h-full flex-col items-center justify-center text-center">
      <h2 className="text-lg font-semibold text-zinc-900 dark:text-zinc-100">
        {title}
      </h2>
      <div className="mt-6 flex items-center gap-3 text-sm text-zinc-700 dark:text-zinc-200">
        <Spinner />
        <span>{progress?.message ?? "Working…"}</span>
      </div>
      <div className="mt-10">
        {provisioningLocked ? (
          <p className="text-sm text-zinc-500 dark:text-zinc-400">
            Please keep the Jetson powered on.
          </p>
        ) : (
          <Button variant="secondary" onClick={cancel}>
            Cancel
          </Button>
        )}
      </div>
    </div>
  );
}