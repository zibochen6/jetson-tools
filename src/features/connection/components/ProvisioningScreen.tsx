import { Button } from "../../../components/Button";
import { useConnectionStore } from "../../../stores/connectionStore";

export function ProvisioningScreen() {
  const progress = useConnectionStore((s) => s.progress);
  const state = useConnectionStore((s) => s.state);
  const cancel = useConnectionStore((s) => s.cancel);
  const provisioningLocked = useConnectionStore((s) => s.provisioningLocked);

  const title = state === "launching_rdp" ? "Opening desktop" : "Preparing your Jetson";

  return (
    <div className="flex h-full flex-col items-center justify-center text-center">
      <div className="jr-breathe flex h-16 w-16 items-center justify-center rounded-full bg-sky-500/10 dark:bg-sky-500/15">
        <span
          className="inline-block h-7 w-7 animate-spin rounded-full border-[3px] border-sky-500/25 border-t-sky-500 dark:border-sky-400/25 dark:border-t-sky-400"
          aria-hidden
        />
      </div>

      <h2 className="mt-5 text-lg font-semibold text-slate-900 dark:text-slate-100">
        {title}
      </h2>

      {/* Indeterminate progress shimmer */}
      <div className="mt-5 h-1 w-56 overflow-hidden rounded-full bg-slate-200 dark:bg-slate-700/60">
        <div className="jr-indeterminate h-full w-2/5 rounded-full bg-gradient-to-r from-sky-500 to-violet-500" />
      </div>

      <div className="mt-4 text-sm text-slate-600 dark:text-slate-300">
        {progress?.message ?? "Working…"}
      </div>

      <div className="mt-10">
        {provisioningLocked ? (
          <p className="text-sm text-slate-500 dark:text-slate-400">
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
