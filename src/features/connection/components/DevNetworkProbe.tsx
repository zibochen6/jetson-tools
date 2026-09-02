import { useState } from "react";
import { useConnectionStore } from "../../../stores/connectionStore";
import { networkProbe, NetworkProbe } from "../tauriService";

/**
 * Dev-only raw TCP probe (same app process as the real connect) — surfaces the
 * OS errno verbatim so we can tell a macOS Local Network (TCC) block apart from
 * a real transport failure. Rendered only when `import.meta.env.DEV`.
 */
export function DevNetworkProbe() {
  const host = useConnectionStore((s) => s.form.host) || "192.168.100.164";
  const [latest, setLatest] = useState<NetworkProbe | null>(null);
  const [busy, setBusy] = useState(false);

  const run = async (port: number) => {
    setBusy(true);
    try {
      setLatest(await networkProbe(host, port));
    } finally {
      setBusy(false);
    }
  };

  return (
    <div className="flex items-center gap-2 text-xs text-zinc-400 dark:text-zinc-500">
      <span className="uppercase tracking-wide">TCP probe</span>
      <button
        onClick={() => void run(22)}
        disabled={busy}
        className="rounded border border-zinc-300 bg-transparent px-1 py-0.5 text-xs dark:border-zinc-600"
      >
        :22
      </button>
      <button
        onClick={() => void run(3389)}
        disabled={busy}
        className="rounded border border-zinc-300 bg-transparent px-1 py-0.5 text-xs dark:border-zinc-600"
      >
        :3389
      </button>
      {latest && (
        <span className="text-zinc-500 dark:text-zinc-400">
          {latest.host}:{latest.port}{" "}
          {latest.connected
            ? "ok"
            : `fail kind=${latest.errorKind} errno=${latest.osErrno} (${latest.detail})`}
        </span>
      )}
    </div>
  );
}