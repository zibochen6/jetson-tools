import { useState } from "react";
import { openUrl } from "@tauri-apps/plugin-opener";
import {
  checkForUpdate,
  downloadAndInstallUpdate,
  isUpdateError,
} from "./updateService";

type Phase =
  | "idle"
  | "checking"
  | "latest"
  | "no-release"
  | "available"
  | "installing"
  | "error";

/**
 * Footer version + GitHub release updater.
 * Dev runs (not bundled .app) can check but not self-install; the UI then
 * links to the release page instead.
 */
export function UpdateChecker({ currentVersion }: { currentVersion: string }) {
  const [phase, setPhase] = useState<Phase>("idle");
  const [latest, setLatest] = useState<string | null>(null);
  const [releaseUrl, setReleaseUrl] = useState<string | null>(null);
  const [assetUrl, setAssetUrl] = useState<string | null>(null);
  const [bundled, setBundled] = useState(false);
  const [error, setError] = useState<string | null>(null);

  async function onCheck() {
    setPhase("checking");
    setError(null);
    try {
      const r = await checkForUpdate();
      setLatest(r.latestVersion);
      setReleaseUrl(r.releaseUrl);
      setAssetUrl(r.appAssetUrl);
      setBundled(r.isBundledApp);
      if (!r.latestVersion) setPhase("no-release");
      else if (r.updateAvailable) setPhase("available");
      else setPhase("latest");
    } catch (e) {
      setError(isUpdateError(e) ? e.message : String(e));
      setPhase("error");
    }
  }

  async function onInstall() {
    if (!assetUrl) return;
    setPhase("installing");
    setError(null);
    try {
      // Never resolves on success — the app quits and relaunches.
      await downloadAndInstallUpdate(assetUrl);
      setPhase("available");
    } catch (e) {
      setError(isUpdateError(e) ? e.message : String(e));
      setPhase("error");
    }
  }

  return (
    <span className="flex items-center gap-2 text-xs text-slate-400 dark:text-slate-500">
      <span>v{currentVersion}</span>
      {phase === "idle" && (
        <button
          onClick={() => void onCheck()}
          className="rounded border border-slate-300 px-1.5 py-0.5 text-slate-500 transition-colors hover:border-sky-400 hover:text-sky-600 dark:border-slate-600 dark:text-slate-400 dark:hover:text-sky-400"
        >
          检查更新
        </button>
      )}
      {phase === "checking" && <span>检查中…</span>}
      {phase === "latest" && <span className="text-emerald-600 dark:text-emerald-400">已是最新</span>}
      {phase === "no-release" && <span>暂无发布版本</span>}
      {(phase === "error" || error) && (
        <span className="max-w-72 text-amber-600 dark:text-amber-400" title={error ?? undefined}>
          {error ?? "检查失败"}
        </span>
      )}
      {phase === "available" && (
        <>
          <span className="text-sky-600 dark:text-sky-400">新版本 v{latest}</span>
          {bundled ? (
            <button
              onClick={() => void onInstall()}
              className="rounded border border-sky-400 px-1.5 py-0.5 text-sky-600 transition-colors hover:bg-sky-50 dark:text-sky-400 dark:hover:bg-sky-950"
            >
              下载并安装
            </button>
          ) : (
            <button
              onClick={() => releaseUrl && void openUrl(releaseUrl)}
              className="rounded border border-sky-400 px-1.5 py-0.5 text-sky-600 transition-colors hover:bg-sky-50 dark:text-sky-400 dark:hover:bg-sky-950"
            >
              打开下载页
            </button>
          )}
        </>
      )}
      {phase === "installing" && (
        <span className="text-sky-600 dark:text-sky-400">下载更新中，完成后自动重启…</span>
      )}
    </span>
  );
}