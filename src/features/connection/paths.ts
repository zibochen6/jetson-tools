// Device-path helpers (identity-v3): classify addresses as LAN vs Tailscale
// and render display labels. Pure functions, no side effects.

/** CGNAT range used by Tailscale (100.64.0.0/10). */
function isTailscale(address: string): boolean {
  const m = /^(\d{1,3})\.(\d{1,3})\./.exec(address.trim());
  if (!m) return false;
  const a = Number(m[1]);
  const b = Number(m[2]);
  return a === 100 && b >= 64 && b <= 127;
}

/** Classify one address: `tailscale` for the CGNAT range, `lan` otherwise. */
export function classifyAddress(address: string): "lan" | "tailscale" {
  return isTailscale(address) ? "tailscale" : "lan";
}

/** Short display label for the address currently in use. */
export function pathLabel(address: string | null | undefined): string {
  if (!address) return "";
  return isTailscale(address)
    ? `Tailscale ${address}`
    : `LAN ${address}`;
}

/**
 * Candidate addresses for a connection: the typed entry host plus every known
 * path of the matching remembered device, deduplicated, typed host first.
 */
export function candidateAddresses(
  entryHost: string,
  knownPaths: string[] | undefined,
): string[] {
  const out: string[] = [];
  const push = (a: string | undefined | null) => {
    const t = a?.trim();
    if (t && !out.includes(t)) out.push(t);
  };
  push(entryHost);
  for (const p of knownPaths ?? []) push(p);
  return out;
}

/** The stable per-device session/tunnel key: `username@deviceId` (or host). */
export function deviceKey(
  username: string,
  deviceId: string | null | undefined,
  host: string,
): string {
  const id = deviceId?.trim();
  return id ? `${username}@${id}` : `${username}@${host}`;
}
