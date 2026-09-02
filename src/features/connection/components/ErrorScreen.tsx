import { Button } from "../../../components/Button";
import { useConnectionStore } from "../../../stores/connectionStore";

export function ErrorScreen() {
  const error = useConnectionStore((s) => s.error);
  const retry = useConnectionStore((s) => s.retry);
  const back = useConnectionStore((s) => s.back);

  if (!error) return null;

  return (
    <div className="mx-auto flex h-full max-w-sm flex-col items-center justify-center text-center">
      <span
        className="flex h-12 w-12 items-center justify-center rounded-full bg-red-100 text-2xl text-red-600 dark:bg-red-900/40 dark:text-red-400"
        aria-hidden
      >
        !
      </span>
      <h2 className="mt-3 text-lg font-semibold text-zinc-900 dark:text-zinc-100">
        {error.title}
      </h2>
      {error.suggestions.length > 0 && (
        <ul className="mt-4 space-y-1 text-sm text-zinc-500 dark:text-zinc-400">
          {error.suggestions.map((s) => (
            <li key={s}>{s}</li>
          ))}
        </ul>
      )}
      <div className="mt-6 flex gap-3">
        <Button onClick={retry}>Retry</Button>
        <Button variant="secondary" onClick={back}>
          Back
        </Button>
      </div>
    </div>
  );
}