import { useState } from "react";
import { Button } from "../../../components/Button";
import { useConnectionStore } from "../../../stores/connectionStore";
import { isValidDisplayName, MAX_DISPLAY_NAME } from "../validation";

/**
 * Mandatory naming screen (identity-v3): a brand-new machine-id must get a
 * display name BEFORE provisioning / opening the desktop. There is no skip —
 * the only exits are a valid name or going back to the connect form.
 */
export function NameDeviceScreen() {
  const device = useConnectionStore((s) => s.device);
  const confirmDeviceName = useConnectionStore((s) => s.confirmDeviceName);
  const back = useConnectionStore((s) => s.back);
  const [name, setName] = useState("");
  const [submitting, setSubmitting] = useState(false);

  const trimmed = name.trim();
  const valid = isValidDisplayName(name);

  const hints = [
    device?.hostname && `主机名 ${device.hostname}`,
    device?.model,
    device?.host,
  ].filter((v): v is string => Boolean(v));

  return (
    <div className="mx-auto flex h-full max-w-sm flex-col justify-center text-center">
      <div
        className="mx-auto mb-4 flex h-14 w-14 items-center justify-center rounded-2xl bg-gradient-to-br from-sky-400 to-violet-500 text-white shadow-lg shadow-sky-500/25"
        aria-hidden
      >
        <svg width="26" height="26" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.8" strokeLinecap="round" strokeLinejoin="round">
          <path d="M12 20h9" />
          <path d="M16.5 3.5a2.121 2.121 0 0 1 3 3L7 19l-4 1 1-4Z" />
        </svg>
      </div>

      <h2 className="text-lg font-semibold text-slate-900 dark:text-slate-100">
        给这台设备起个名字
      </h2>
      <p className="mt-1.5 text-sm text-slate-500 dark:text-slate-400">
        首次连接的新设备需要一个名字。之后这台设备（无论用局域网还是
        Tailscale 地址进入）都只会显示这个名字。
      </p>

      {hints.length > 0 && (
        <p className="mt-3 truncate text-xs text-slate-400 dark:text-slate-500">
          {hints.join(" · ")}
        </p>
      )}

      <form
        className="mt-6 text-left"
        onSubmit={(e) => {
          e.preventDefault();
          if (!valid || submitting) return;
          setSubmitting(true);
          void confirmDeviceName(name).then((ok) => {
            if (!ok) setSubmitting(false);
          });
        }}
      >
        <label
          htmlFor="device-name"
          className="mb-1.5 block text-sm font-medium text-slate-700 dark:text-slate-300"
        >
          设备名称 <span className="text-red-500">*</span>
        </label>
        <input
          id="device-name"
          className={
            "w-full rounded-lg border border-slate-300 bg-white px-3.5 py-2.5 text-sm text-slate-900 " +
            "placeholder:text-slate-400 transition-all duration-150 " +
            "focus:border-sky-500 focus:outline-none focus:ring-2 focus:ring-sky-500/25 " +
            "dark:border-slate-600/70 dark:bg-slate-900/50 dark:text-slate-100 dark:placeholder:text-slate-500"
          }
          value={name}
          onChange={(e) => setName(e.target.value)}
          placeholder="例如：实验室 robotics"
          maxLength={MAX_DISPLAY_NAME}
          autoFocus
          autoComplete="off"
        />
        {!valid && trimmed.length > 0 && (
          <p className="mt-1.5 text-xs text-red-600 dark:text-red-400">
            名称不能为空，最长 {MAX_DISPLAY_NAME} 个字符。
          </p>
        )}

        <Button type="submit" disabled={!valid || submitting} className="mt-4 w-full">
          {submitting ? "正在连接…" : "继续"}
        </Button>
      </form>

      <button
        type="button"
        onClick={back}
        className="mt-4 text-xs font-medium text-slate-400 transition-colors duration-150 hover:text-slate-600 dark:text-slate-500 dark:hover:text-slate-300"
      >
        返回
      </button>
    </div>
  );
}
