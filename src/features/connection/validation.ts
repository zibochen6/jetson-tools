// Pure input validation for the connection form. No React, no side effects.

const IPV4_RE =
  /^(25[0-5]|2[0-4]\d|1\d\d|[1-9]?\d)(\.(25[0-5]|2[0-4]\d|1\d\d|[1-9]?\d)){3}$/;

// Hostname: RFC-1123-ish labels, optional trailing dot, allows single label
// (e.g. "jetson" or "jetson.local"). Disallows spaces and empty labels.
const HOSTNAME_RE =
  /^(?=.{1,253}$)([a-zA-Z0-9]([a-zA-Z0-9-]{0,61}[a-zA-Z0-9])?\.)*[a-zA-Z0-9]([a-zA-Z0-9-]{0,61}[a-zA-Z0-9])?\.?$/;

export function isValidIpv4(host: string): boolean {
  return IPV4_RE.test(host.trim());
}

export function isValidHostname(host: string): boolean {
  return HOSTNAME_RE.test(host.trim());
}

/** Accept either an IPv4 address or a hostname (MVP: no IPv6). */
export function isValidHost(host: string): boolean {
  const h = host.trim();
  if (!h) return false;
  return isValidIpv4(h) || isValidHostname(h);
}

export function isValidUsername(username: string): boolean {
  return username.trim().length > 0;
}

export function isValidPassword(password: string): boolean {
  return password.length > 0;
}

export interface FormErrors {
  host?: string;
  username?: string;
  password?: string;
}

export interface ConnectionForm {
  host: string;
  username: string;
  password: string;
  remember: boolean;
}

/**
 * A remembered device with a usable stored password exempts the form from the
 * empty-password rule: the backend resolves the secret itself (V0.3).
 */
export interface SavedPasswordSource {
  host: string;
  username: string;
  hasPassword?: boolean;
}

function matchesSaved(form: ConnectionForm, saved?: SavedPasswordSource | null): boolean {
  return (
    !!saved &&
    saved.hasPassword === true &&
    saved.host === form.host.trim() &&
    saved.username === form.username.trim()
  );
}

export function validateConnectionForm(
  form: ConnectionForm,
  saved?: SavedPasswordSource | null,
): FormErrors {
  const errors: FormErrors = {};
  if (!isValidHost(form.host)) {
    errors.host = "Enter a valid IP address or hostname.";
  }
  if (!isValidUsername(form.username)) {
    errors.username = "Username is required.";
  }
  if (!isValidPassword(form.password) && !matchesSaved(form, saved)) {
    errors.password = "Password is required.";
  }
  return errors;
}

export function isFormValid(
  form: ConnectionForm,
  saved?: SavedPasswordSource | null,
): boolean {
  return Object.keys(validateConnectionForm(form, saved)).length === 0;
}