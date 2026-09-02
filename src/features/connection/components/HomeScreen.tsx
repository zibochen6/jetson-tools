import { useConnectionStore } from "../../../stores/connectionStore";
import { ConnectionForm } from "./ConnectionForm";

export function HomeScreen() {
  const savedDevice = useConnectionStore((s) => s.savedDevice);
  const forgetDevice = useConnectionStore((s) => s.forgetDevice);

  return (
    <div className="flex h-full flex-col justify-center">
      <header className="mb-8 text-center">
        <h1 className="text-xl font-semibold text-zinc-900 dark:text-zinc-100">
          Jetson Remote
        </h1>
        <p className="mt-1 text-sm text-zinc-500 dark:text-zinc-400">
          Connect to your Jetson
        </p>
      </header>

      {savedDevice && (
        <div className="mb-4 flex items-center justify-between rounded-md border border-zinc-200 bg-white px-3 py-2 text-sm dark:border-zinc-700 dark:bg-zinc-800">
          <span className="truncate text-zinc-600 dark:text-zinc-300">
            Last connected:{" "}
            <span className="font-medium text-zinc-900 dark:text-zinc-100">
              {savedDevice.username}@{savedDevice.host}
            </span>
          </span>
          <button
            type="button"
            onClick={() => void forgetDevice()}
            className="ml-3 shrink-0 rounded px-2 py-1 text-xs font-medium text-zinc-500 hover:bg-zinc-100 hover:text-zinc-700 dark:text-zinc-400 dark:hover:bg-zinc-700 dark:hover:text-zinc-200"
          >
            Forget
          </button>
        </div>
      )}

      <ConnectionForm />
    </div>
  );
}