import { invoke } from "@tauri-apps/api/core";

export interface AppInfo {
  name: string;
  version: string;
}

/**
 * Tauri IPC smoke boundary. When the app runs outside Tauri (e.g. `vite dev`
 * in a plain browser) these return null instead of throwing.
 */
export async function getAppInfo(): Promise<AppInfo | null> {
  try {
    return await invoke<AppInfo>("app_info");
  } catch {
    return null;
  }
}

export async function healthCheck(): Promise<string | null> {
  try {
    return await invoke<string>("health_check");
  } catch {
    return null;
  }
}