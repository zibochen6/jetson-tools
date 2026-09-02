import { useState } from "react";
import { useConnectionStore } from "../../../stores/connectionStore";
import { useSessionsStore, SessionPhase } from "../../../stores/sessionsStore";
import { ConnectionForm } from "./ConnectionForm";
import { Button } from "../../../components/Button";

/**
 * Home screen — Option C "Device Grid": brand hero + one card per connected
 * Jetson + dashed "add device" card revealing the connection form. With zero
 * sessions the layout degrades to the original single-form hero, so the
 * first-run experience is unchanged.
 */
export function HomeScreen() {
  const savedDevice = useConnectionStore((s) => s.savedDevice);
  const forgetDevice = useConnectionStore((s) => s.forgetDevice);
  const setForm = useConnectionStore((s) => s.setForm);
  const connect = useConnectionStore((s) => s.connect);

  const order = useSessionsStore((s) => s.order);
  const sessions = useSessionsStore((s) => s.sessions);
  const focusTab = useSessionsStore((s) => s.focusTab);
  const closeTab = useSessionsStore((s) => s.closeTab);

  const hasSessions = order.length > 0;
  const [adding, setAdding] = useState(!hasSessions);

  const quickConnect = () => {
    if (!savedDevice) return;
    setForm({
      host: savedDevice.host,
      username: savedDevice.username,
      password: "",
      remember: true,
    });
    void connect();
  };

  /* ---- zero sessions: the original single-device hero (unchanged UX) ---- */
  if (!hasSessions) {
    return (
      <div className="mx-auto flex h-full max-w-sm flex-col justify-center">
        <BrandHero />

        {savedDevice && (
          <div className="jr-enter mb-4 flex items-center gap-3 rounded-xl border border-slate-200 bg-white px-4 py-3 transition-all duration-200 hover:-translate-y-0.5 hover:shadow-lg hover:shadow-slate-900/5 dark:border-slate-700/70 dark:bg-slate-800/60 dark:hover:border-slate-600 dark:hover:shadow-black/30">
            <span
              className={`h-2 w-2 shrink-0 rounded-full ${
                savedDevice.hasPassword
                  ? "jr-dot-live bg-emerald-500"
                  : "bg-slate-400 dark:bg-slate-500"
              }`}
              aria-hidden
            />
            <div className="min-w-0 flex-1">
              <div className="truncate text-sm font-medium text-slate-700 dark:text-slate-200">
                {savedDevice.username}@{savedDevice.host}
              </div>
              <div className="text-xs text-slate-400 dark:text-slate-500">
                上次连接
              </div>
            </div>
            {savedDevice.hasPassword && (
              <button
                type="button"
                onClick={quickConnect}
                className="shrink-0 rounded-lg bg-sky-500/10 px-3 py-1.5 text-xs font-semibold text-sky-600 transition-all duration-150 hover:bg-sky-500/20 active:scale-[0.98] dark:text-sky-400"
              >
                快速连接
              </button>
            )}
            <button
              type="button"
              onClick={() => void forgetDevice()}
              title="忘记此设备"
              className="shrink-0 rounded-md px-2 py-1 text-xs font-medium text-slate-400 transition-colors duration-150 hover:bg-slate-100 hover:text-slate-600 dark:hover:bg-slate-700/60 dark:hover:text-slate-300"
            >
              忘记
            </button>
          </div>
        )}

        <div className="jr-enter rounded-2xl border border-slate-200/80 bg-white/70 p-6 shadow-sm backdrop-blur-sm dark:border-slate-700/60 dark:bg-slate-800/40">
          <ConnectionForm />
        </div>
      </div>
    );
  }

  /* ---- one or more live sessions: the device grid ---- */
  return (
    <div className="mx-auto flex h-full max-w-3xl flex-col">
      <div className="mb-6 flex items-center justify-between">
        <div className="flex items-center gap-3">
          <div
            className="flex h-10 w-10 items-center justify-center rounded-xl bg-gradient-to-br from-sky-400 to-violet-500 text-white shadow-md shadow-sky-500/20"
            aria-hidden
          >
            <svg width="19" height="19" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.8" strokeLinecap="round" strokeLinejoin="round">
              <rect x="2" y="3" width="20" height="14" rx="2" />
              <path d="M8 21h8M12 17v4" />
            </svg>
          </div>
          <div>
            <h1 className="text-lg font-bold tracking-tight text-slate-900 dark:text-slate-50">
              Jetson Remote
            </h1>
            <p className="text-xs text-slate-500 dark:text-slate-400">
              {order.length} 台设备已连接 · 点击卡片切换桌面
            </p>
          </div>
        </div>
        {!adding && (
          <Button onClick={() => setAdding(true)}>＋ 连接新设备</Button>
        )}
      </div>

      <div className="grid grid-cols-1 gap-3 sm:grid-cols-2 lg:grid-cols-3">
        {order.map((id) => {
          const s = sessions[id];
          if (!s) return null;
          return (
            <SessionCard
              key={id}
              phase={s.phase}
              title={s.device?.hostname ?? s.host}
              subtitle={`${s.username}@${s.host}`}
              meta={s.device?.model}
              onOpen={() => focusTab(id)}
              onClose={() => closeTab(id)}
            />
          );
        })}

        {/* Add-device card */}
        <button
          type="button"
          onClick={() => setAdding(true)}
          className="jr-enter flex min-h-32 flex-col items-center justify-center gap-2 rounded-xl border border-dashed border-slate-300 text-slate-400 transition-all duration-200 hover:-translate-y-0.5 hover:border-sky-400 hover:text-sky-500 active:scale-[0.99] dark:border-slate-600 dark:text-slate-500 dark:hover:border-sky-500/70 dark:hover:text-sky-400"
        >
          <svg width="22" height="22" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" aria-hidden>
            <path d="M12 5v14M5 12h14" />
          </svg>
          <span className="text-sm font-medium">添加设备</span>
        </button>
      </div>

      {adding && (
        <div className="jr-enter mx-auto mt-6 w-full max-w-sm rounded-2xl border border-slate-200/80 bg-white/70 p-6 shadow-sm backdrop-blur-sm dark:border-slate-700/60 dark:bg-slate-800/40">
          <div className="mb-4 flex items-center justify-between">
            <h2 className="text-sm font-semibold text-slate-800 dark:text-slate-100">
              连接新设备
            </h2>
            <button
              type="button"
              onClick={() => setAdding(false)}
              className="rounded-md px-2 py-1 text-xs font-medium text-slate-400 transition-colors duration-150 hover:bg-slate-100 hover:text-slate-600 dark:hover:bg-slate-700/60 dark:hover:text-slate-300"
            >
              收起
            </button>
          </div>
          <ConnectionForm />
        </div>
      )}
    </div>
  );
}

function BrandHero() {
  return (
    <header className="jr-enter-slow mb-8 flex flex-col items-center text-center">
      <div
        className="mb-4 flex h-14 w-14 items-center justify-center rounded-2xl bg-gradient-to-br from-sky-400 to-violet-500 text-white shadow-lg shadow-sky-500/25"
        aria-hidden
      >
        <svg width="26" height="26" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.8" strokeLinecap="round" strokeLinejoin="round">
          <rect x="2" y="3" width="20" height="14" rx="2" />
          <path d="M8 21h8M12 17v4" />
        </svg>
      </div>
      <h1 className="text-2xl font-bold tracking-tight text-slate-900 dark:text-slate-50">
        Jetson Remote
      </h1>
      <p className="mt-1.5 text-sm text-slate-500 dark:text-slate-400">
        连接局域网中的 Jetson 桌面
      </p>
    </header>
  );
}

const CARD_ACTION: Record<SessionPhase, { label: string; disabled: boolean }> = {
  running: { label: "切换到桌面", disabled: false },
  launching: { label: "正在打开…", disabled: true },
  ready: { label: "打开桌面", disabled: false },
  error: { label: "重试连接", disabled: false },
};

const CARD_DOT: Record<SessionPhase, string> = {
  running: "jr-dot-live bg-emerald-500",
  launching: "animate-pulse bg-amber-400",
  ready: "bg-slate-400 dark:bg-slate-500",
  error: "bg-red-500",
};

/** Adapted catalog #5 (Feature Card): icon chip + title + meta, hover lift. */
function SessionCard({
  phase,
  title,
  subtitle,
  meta,
  onOpen,
  onClose,
}: {
  phase: SessionPhase;
  title: string;
  subtitle: string;
  meta?: string;
  onOpen: () => void;
  onClose: () => void;
}) {
  const action = CARD_ACTION[phase];
  return (
    <div
      className="jr-enter group flex cursor-pointer flex-col gap-3 rounded-xl border border-slate-200 bg-white p-4 text-left transition-all duration-200 hover:-translate-y-0.5 hover:shadow-lg hover:shadow-slate-900/5 dark:border-slate-700/70 dark:bg-slate-800/60 dark:hover:border-slate-600 dark:hover:shadow-black/30"
      onClick={onOpen}
      role="button"
      tabIndex={0}
      onKeyDown={(e) => {
        if (e.key === "Enter" || e.key === " ") {
          e.preventDefault();
          onOpen();
        }
      }}
    >
      <div className="flex items-center gap-3">
        <span
          className="flex h-10 w-10 shrink-0 items-center justify-center rounded-lg bg-sky-50 text-sky-600 dark:bg-sky-500/10 dark:text-sky-400"
          aria-hidden
        >
          <svg width="20" height="20" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.8" strokeLinecap="round" strokeLinejoin="round">
            <rect x="2" y="3" width="20" height="14" rx="2" />
            <path d="M8 21h8M12 17v4" />
          </svg>
        </span>
        <div className="min-w-0 flex-1">
          <div className="flex items-center gap-2">
            <span className="truncate text-sm font-semibold text-slate-900 dark:text-slate-100">
              {title}
            </span>
            <span className={`h-2 w-2 shrink-0 rounded-full ${CARD_DOT[phase]}`} aria-hidden />
          </div>
          <div className="truncate text-xs text-slate-500 dark:text-slate-400">
            {subtitle}
          </div>
        </div>
        <button
          type="button"
          aria-label={`断开 ${title}`}
          onClick={(e) => {
            e.stopPropagation();
            onClose();
          }}
          className="flex h-6 w-6 shrink-0 items-center justify-center rounded-md text-slate-300 opacity-0 transition-all duration-150 hover:bg-red-500/15 hover:text-red-500 group-hover:opacity-100 dark:text-slate-600 dark:hover:text-red-400"
        >
          <svg width="11" height="11" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2.5" strokeLinecap="round" aria-hidden>
            <path d="M18 6 6 18M6 6l12 12" />
          </svg>
        </button>
      </div>

      {meta && (
        <div className="truncate text-xs text-slate-400 dark:text-slate-500">
          {meta}
        </div>
      )}

      <span
        className={`mt-auto inline-flex items-center justify-center rounded-lg px-3 py-2 text-xs font-semibold transition-all duration-150 ${
          action.disabled
            ? "cursor-not-allowed bg-slate-100 text-slate-400 dark:bg-slate-700/40 dark:text-slate-500"
            : "bg-sky-500/10 text-sky-600 group-hover:bg-sky-500/20 dark:text-sky-400"
        }`}
      >
        {action.label}
      </span>
    </div>
  );
}
