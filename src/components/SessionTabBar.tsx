import { useSessionsStore, SessionPhase } from "../stores/sessionsStore";
import { useConnectionStore } from "../stores/connectionStore";

/** Must equal Rust `commands::rdp::SESSION_TAB_BAR_INSET` (points). */
export const TAB_BAR_H = 44;

const DOT: Record<SessionPhase, string> = {
  running: "jr-dot-live bg-emerald-500",
  launching: "animate-pulse bg-amber-400",
  ready: "bg-slate-400 dark:bg-slate-500",
  error: "bg-red-500",
};

function labelFor(host: string, hostname?: string): string {
  return hostname && hostname.length > 0 ? hostname : host;
}

/**
 * Multi-device tab bar (V0.4, layout Option C). Rendered in the 44px strip
 * the native desktop views leave free at the top of the window, so it stays
 * clickable while a desktop is on screen — that is the quick-switch surface.
 */
export function SessionTabBar() {
  const sessions = useSessionsStore((s) => s.sessions);
  const order = useSessionsStore((s) => s.order);
  const activeId = useSessionsStore((s) => s.activeId);
  const focusTab = useSessionsStore((s) => s.focusTab);
  const closeTab = useSessionsStore((s) => s.closeTab);
  const showOverview = useSessionsStore((s) => s.showOverview);
  const wizardState = useConnectionStore((s) => s.state);

  const overviewActive = activeId === null;
  const wizardActive = wizardState !== "idle" && wizardState !== "desktop_opened";

  return (
    <div
      className="flex h-11 shrink-0 items-center gap-1 overflow-x-auto border-b border-slate-200/80 bg-white/75 px-2 backdrop-blur-md dark:border-slate-800 dark:bg-slate-900/75"
      role="tablist"
      aria-label="已连接的 Jetson"
    >
      {/* Overview chip — always returns to the device grid */}
      <button
        type="button"
        role="tab"
        aria-selected={overviewActive}
        onClick={showOverview}
        className={`flex h-8 shrink-0 items-center gap-1.5 rounded-lg px-3 text-xs font-medium transition-all duration-150 active:scale-[0.98] ${
          overviewActive && !wizardActive
            ? "bg-slate-200/80 text-slate-800 dark:bg-slate-700/70 dark:text-slate-100"
            : "text-slate-500 hover:bg-slate-200/50 hover:text-slate-700 dark:text-slate-400 dark:hover:bg-slate-800 dark:hover:text-slate-200"
        }`}
      >
        <svg width="13" height="13" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round" aria-hidden>
          <path d="m3 9 9-7 9 7v11a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2z" />
        </svg>
        设备总览
      </button>

      <span className="mx-1 h-5 w-px shrink-0 bg-slate-200 dark:bg-slate-700/70" aria-hidden />

      {order.map((id) => {
        const session = sessions[id];
        if (!session) return null;
        const active = activeId === id && !wizardActive;
        return (
          <div
            key={id}
            className={`group flex h-8 shrink-0 cursor-pointer items-center gap-2 rounded-lg pl-3 pr-1.5 text-xs transition-all duration-150 ${
              active
                ? "bg-slate-200/80 text-slate-900 shadow-sm dark:bg-slate-700/70 dark:text-slate-50"
                : "text-slate-500 hover:bg-slate-200/50 hover:text-slate-700 dark:text-slate-400 dark:hover:bg-slate-800 dark:hover:text-slate-200"
            }`}
            role="tab"
            aria-selected={active}
            onClick={() => focusTab(id)}
            title={`${session.username}@${session.host}`}
          >
            <span
              className={`h-2 w-2 shrink-0 rounded-full ${DOT[session.phase]}`}
              aria-hidden
            />
            <span className="max-w-36 truncate font-medium">
              {labelFor(session.host, session.device?.hostname)}
            </span>
            <button
              type="button"
              aria-label={`断开 ${session.host}`}
              onClick={(e) => {
                e.stopPropagation();
                closeTab(id);
              }}
              className="flex h-5 w-5 items-center justify-center rounded-md text-slate-400 opacity-60 transition-all duration-150 hover:bg-red-500/15 hover:text-red-500 group-hover:opacity-100 dark:text-slate-500 dark:hover:text-red-400"
            >
              <svg width="10" height="10" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2.5" strokeLinecap="round" aria-hidden>
                <path d="M18 6 6 18M6 6l12 12" />
              </svg>
            </button>
          </div>
        );
      })}

      {/* Add another device — the overview shows the connect form */}
      <button
        type="button"
        onClick={showOverview}
        title="连接新设备"
        className="ml-1 flex h-8 w-8 shrink-0 items-center justify-center rounded-lg border border-dashed border-slate-300 text-slate-400 transition-all duration-150 hover:border-sky-400 hover:text-sky-500 active:scale-[0.98] dark:border-slate-600 dark:text-slate-500 dark:hover:border-sky-500/70 dark:hover:text-sky-400"
      >
        <svg width="13" height="13" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2.5" strokeLinecap="round" aria-hidden>
          <path d="M12 5v14M5 12h14" />
        </svg>
      </button>
    </div>
  );
}
