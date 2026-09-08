// Rust `ErrorCode` → frontend `ConnectionErrorCode` mapping
// (Reliability Harness Phase 0/1).
//
// The backend owns the stable error vocabulary; the frontend must never
// derive logic from human-readable strings, only from codes. This table is
// the single source of truth for the translation, kept pure and testable.

import type { ConnectionErrorCode } from "./types";

/**
 * Stable backend codes emitted by `src-tauri/src/error_code.rs`.
 * Keep in sync with that enum (append-only on both sides).
 */
export type BackendErrorCode =
  | "ssh_timeout"
  | "ssh_auth_failed"
  | "ssh_host_key_changed"
  | "device_unreachable"
  | "not_a_jetson"
  | "sudo_failed"
  | "provision_step_failed"
  | "tunnel_start_failed"
  | "tunnel_target_unreachable"
  | "loopback_ssh_failed"
  | "rdp_connect_failed"
  | "rdp_engine_crashed"
  | "unknown";

const BACKEND_TO_FRONTEND: Record<BackendErrorCode, ConnectionErrorCode> = {
  ssh_timeout: "ssh_timeout",
  ssh_auth_failed: "auth_failed",
  ssh_host_key_changed: "ssh_host_key_changed",
  device_unreachable: "device_unreachable",
  not_a_jetson: "not_jetson",
  sudo_failed: "sudo_required",
  provision_step_failed: "provision_failed",
  tunnel_start_failed: "tunnel_failed",
  tunnel_target_unreachable: "tunnel_failed",
  loopback_ssh_failed: "tunnel_failed",
  rdp_connect_failed: "rdp_connection_failed",
  rdp_engine_crashed: "rdp_engine_crashed",
  unknown: "unknown",
};

/**
 * Translate a backend error code. Unknown future codes degrade to
 * `unknown` (forward-compatible): the UI still renders, humans still see a
 * safe message, and no code can carry credentials through this path.
 */
export function mapBackendError(code: string): ConnectionErrorCode {
  return (
    BACKEND_TO_FRONTEND[code as BackendErrorCode] ?? "unknown"
  );
}