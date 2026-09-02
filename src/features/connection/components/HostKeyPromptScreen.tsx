import { Button } from "../../../components/Button";
import { useConnectionStore } from "../../../stores/connectionStore";

/** TOFU trust prompt: unknown key (Trust & Connect) or changed key (Replace). */
export function HostKeyPromptScreen() {
  const state = useConnectionStore((s) => s.state);
  const hostKey = useConnectionStore((s) => s.hostKey);
  const previousKey = useConnectionStore((s) => s.previousKey);
  const trustKey = useConnectionStore((s) => s.trustKey);
  const replaceKey = useConnectionStore((s) => s.replaceKey);
  const back = useConnectionStore((s) => s.back);

  const changed = state === "host_key_changed";

  return (
    <div className="mx-auto flex h-full max-w-sm flex-col justify-center text-center">
      <h2 className="text-lg font-semibold text-zinc-900 dark:text-zinc-100">
        {changed ? "SSH identity changed" : "Trust this Jetson?"}
      </h2>

      {!changed && hostKey && (
        <div className="mt-4">
          <Fingerprint
            label={`${hostKey.host}:${hostKey.port} (${hostKey.algorithm})`}
            fingerprint={hostKey.fingerprint}
          />
        </div>
      )}

      {changed && (
        <div className="mt-4 space-y-3 text-left">
          <p className="text-sm text-zinc-500 dark:text-zinc-400">
            The SSH fingerprint for this Jetson is different from the one
            previously trusted. This can happen after reinstalling the Jetson,
            but it may also indicate a security problem.
          </p>
          <Fingerprint
            label="Previous"
            fingerprint={previousKey?.fingerprint ?? "unknown"}
          />
          <Fingerprint
            label="Current"
            fingerprint={hostKey?.fingerprint ?? "unknown"}
          />
        </div>
      )}

      <div className="mt-6 flex justify-center gap-3">
        {changed ? (
          <>
            <Button onClick={replaceKey}>Replace Trusted Key</Button>
            <Button variant="secondary" onClick={back}>
              Cancel
            </Button>
          </>
        ) : (
          <>
            <Button onClick={trustKey}>Trust & Connect</Button>
            <Button variant="secondary" onClick={back}>
              Cancel
            </Button>
          </>
        )}
      </div>
    </div>
  );
}

function Fingerprint({
  label,
  fingerprint,
}: {
  label: string;
  fingerprint: string;
}) {
  return (
    <div className="rounded-md border border-zinc-200 bg-white px-3 py-2 text-left dark:border-zinc-700 dark:bg-zinc-800">
      <div className="text-xs text-zinc-500 dark:text-zinc-400">{label}</div>
      <code className="mt-0.5 block break-all text-xs text-zinc-800 dark:text-zinc-200">
        {fingerprint}
      </code>
    </div>
  );
}