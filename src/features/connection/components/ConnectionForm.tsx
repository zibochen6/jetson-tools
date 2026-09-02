import { ReactNode, useMemo, useState } from "react";
import { Button } from "../../../components/Button";
import { useConnectionStore } from "../../../stores/connectionStore";
import { validateConnectionForm } from "../validation";

/** Catalog-style input (adapted #14): subtle surface, focus ring in accent. */
const inputCls =
  "w-full rounded-lg border border-slate-300 bg-white px-3.5 py-2.5 text-sm text-slate-900 " +
  "placeholder:text-slate-400 transition-all duration-150 " +
  "focus:border-sky-500 focus:outline-none focus:ring-2 focus:ring-sky-500/25 " +
  "dark:border-slate-600/70 dark:bg-slate-900/50 dark:text-slate-100 dark:placeholder:text-slate-500";

function Field({
  label,
  htmlFor,
  error,
  children,
}: {
  label: string;
  htmlFor: string;
  error?: string;
  children: ReactNode;
}) {
  return (
    <div>
      <label
        htmlFor={htmlFor}
        className="mb-1.5 block text-sm font-medium text-slate-700 dark:text-slate-300"
      >
        {label}
      </label>
      {children}
      {error && (
        <p className="mt-1.5 text-xs text-red-600 dark:text-red-400">{error}</p>
      )}
    </div>
  );
}

export function ConnectionForm() {
  const form = useConnectionStore((s) => s.form);
  const setForm = useConnectionStore((s) => s.setForm);
  const connect = useConnectionStore((s) => s.connect);
  const savedDevice = useConnectionStore((s) => s.savedDevice);
  const [showPassword, setShowPassword] = useState(false);

  const errors = useMemo(
    () => validateConnectionForm(form, savedDevice),
    [form, savedDevice],
  );
  const valid = Object.keys(errors).length === 0;

  // The backend can reuse the stored password: blank field = use Keychain.
  const usingSavedPassword =
    savedDevice?.hasPassword === true &&
    savedDevice.host === form.host.trim() &&
    savedDevice.username === form.username.trim() &&
    form.password === "";

  return (
    <form
      className="space-y-4"
      onSubmit={(e) => {
        e.preventDefault();
        if (valid) void connect();
      }}
    >
      <Field label="Jetson IP" htmlFor="host" error={errors.host}>
        <input
          id="host"
          className={inputCls}
          value={form.host}
          onChange={(e) => setForm({ host: e.target.value })}
          placeholder="192.168.1.100 or jetson.local"
          autoFocus
          autoCapitalize="off"
          autoCorrect="off"
          spellCheck={false}
        />
      </Field>

      <Field label="Username" htmlFor="username" error={errors.username}>
        <input
          id="username"
          className={inputCls}
          value={form.username}
          onChange={(e) => setForm({ username: e.target.value })}
          placeholder="seeed"
          autoCapitalize="off"
          autoCorrect="off"
          spellCheck={false}
        />
      </Field>

      <Field label="Password" htmlFor="password" error={errors.password}>
        <div className="relative">
          <input
            id="password"
            className={`${inputCls} pr-16`}
            type={showPassword ? "text" : "password"}
            value={form.password}
            onChange={(e) => setForm({ password: e.target.value })}
            placeholder={usingSavedPassword ? "Using saved password" : undefined}
            autoCapitalize="off"
            autoCorrect="off"
            spellCheck={false}
          />
          <button
            type="button"
            tabIndex={-1}
            onClick={() => setShowPassword((v) => !v)}
            className="absolute inset-y-0 right-3 text-xs font-medium text-slate-400 transition-colors duration-150 hover:text-slate-600 dark:text-slate-500 dark:hover:text-slate-300"
          >
            {showPassword ? "Hide" : "Show"}
          </button>
        </div>
      </Field>

      <label className="flex cursor-pointer items-center gap-2 text-sm text-slate-600 transition-colors duration-150 hover:text-slate-800 dark:text-slate-300 dark:hover:text-slate-100">
        <input
          type="checkbox"
          checked={form.remember}
          onChange={(e) => setForm({ remember: e.target.checked })}
        />
        Remember this device
      </label>

      {usingSavedPassword && (
        <p className="text-xs text-slate-500 dark:text-slate-400">
          Leaving the password blank signs in with the password saved on this Mac.
        </p>
      )}

      <Button type="submit" disabled={!valid} className="w-full">
        Connect
      </Button>
    </form>
  );
}
