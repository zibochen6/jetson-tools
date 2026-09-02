import { JetsonDevice } from "../features/connection/types";

/**
 * Adapted from catalog component #5 (Feature Card): icon chip + title +
 * metadata, hover lift. Props unchanged from the original component.
 */
export function DeviceCard({ device }: { device: JetsonDevice }) {
  const details = [
    device.model,
    device.jetpackVersion && `JetPack ${device.jetpackVersion}`,
    device.ubuntuVersion && `Ubuntu ${device.ubuntuVersion}`,
    device.l4tVersion && `L4T ${device.l4tVersion}`,
    device.architecture,
  ].filter((v): v is string => Boolean(v));

  return (
    <div className="flex items-center gap-3 rounded-xl border border-slate-200 bg-white px-4 py-3 text-left shadow-none transition-all duration-200 hover:-translate-y-0.5 hover:shadow-lg hover:shadow-slate-900/5 dark:border-slate-700/70 dark:bg-slate-800/60 dark:hover:border-slate-600 dark:hover:shadow-black/30">
      <span
        className="flex h-10 w-10 shrink-0 items-center justify-center rounded-lg bg-sky-50 text-sky-600 dark:bg-sky-500/10 dark:text-sky-400"
        aria-hidden
      >
        <svg width="20" height="20" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.8" strokeLinecap="round" strokeLinejoin="round">
          <rect x="2" y="3" width="20" height="14" rx="2" />
          <path d="M8 21h8M12 17v4" />
        </svg>
      </span>
      <div className="min-w-0">
        <div className="truncate text-sm font-semibold text-slate-900 dark:text-slate-100">
          {device.hostname ?? device.host}
        </div>
        {details.length > 0 && (
          <div className="mt-0.5 truncate text-xs text-slate-500 dark:text-slate-400">
            {details.join(" · ")}
          </div>
        )}
      </div>
    </div>
  );
}
