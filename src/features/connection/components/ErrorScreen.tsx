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
        className="flex h-14 w-14 items-center justify-center rounded-full bg-red-500/10 text-2xl text-red-600 dark:bg-red-500/15 dark:text-red-400"
        aria-hidden
      >
        <svg width="26" height="26" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
          <path d="M10.29 3.86 1.82 18a2 2 0 0 0 1.71 3h16.94a2 2 0 0 0 1.71-3L13.71 3.86a2 2 0 0 0-3.42 0z" />
          <line x1="12" y1="9" x2="12" y2="13" />
          <line x1="12" y1="17" x2="12.01" y2="17" />
        </svg>
      </span>
      <h2 className="mt-4 text-lg font-semibold text-slate-900 dark:text-slate-100">
        {error.title}
      </h2>
      {error.suggestions.length > 0 && (
        <ul className="mt-4 space-y-1.5 text-sm leading-relaxed text-slate-500 dark:text-slate-400">
          {error.suggestions.map((s) => (
            <li key={s}>{s}</li>
          ))}
        </ul>
      )}
      {error.detail && (
        <p
          className="mt-3 max-w-full truncate rounded-md bg-slate-100 px-2.5 py-1.5 font-mono text-[11px] text-slate-500 dark:bg-slate-800/70 dark:text-slate-400"
          title={error.detail}
        >
          {error.detail}
        </p>
      )}
      <div className="mt-7 flex gap-3">
        <Button onClick={retry}>Retry</Button>
        <Button variant="secondary" onClick={back}>
          Back
        </Button>
      </div>
    </div>
  );
}
