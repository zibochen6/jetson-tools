import { MockScenario } from "../service";
import { useConnectionStore } from "../../../stores/connectionStore";

const SCENARIOS: MockScenario[] = [
  "success",
  "ssh_timeout",
  "auth_failed",
  "not_jetson",
  "provision_failed",
  "rdp_failed",
  "launch_failed",
  "host_key_unknown",
  "host_key_changed",
  "provision_needed",
  "sudo_denied",
  "verification_failed",
];

/** Dev-only backend toggle + mock-outcome selector; gated by the caller. */
export function DevScenarioPicker() {
  const scenario = useConnectionStore((s) => s.scenario);
  const setScenario = useConnectionStore((s) => s.setScenario);
  const mode = useConnectionStore((s) => s.mode);
  const setMode = useConnectionStore((s) => s.setMode);

  return (
    <div className="flex items-center gap-4 text-xs text-zinc-400 dark:text-zinc-500">
      <label className="flex items-center gap-2">
        <span className="uppercase tracking-wide">Mode</span>
        <select
          value={mode}
          onChange={(e) => setMode(e.target.value as "real" | "mock")}
          className="rounded border border-zinc-300 bg-transparent px-1 py-0.5 text-xs dark:border-zinc-600"
        >
          <option value="real">Real</option>
          <option value="mock">Mock</option>
        </select>
      </label>
      <label className="flex items-center gap-2">
        <span className="uppercase tracking-wide">Scenario</span>
        <select
          value={scenario}
          onChange={(e) => setScenario(e.target.value as MockScenario)}
          className="rounded border border-zinc-300 bg-transparent px-1 py-0.5 text-xs dark:border-zinc-600"
        >
          {SCENARIOS.map((s) => (
            <option key={s} value={s}>
              {s}
            </option>
          ))}
        </select>
      </label>
    </div>
  );
}