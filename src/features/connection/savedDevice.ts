// Remembered-device gateway (V0.3): loads/saves/forgets the device memory
// through Tauri commands. The stored PASSWORD never comes back through this
// interface — `hasPassword` is a capability flag only (PRD §67).

import { invoke } from "@tauri-apps/api/core";

/** Shape of the remembered device as the frontend may know it. NO password. */
export interface SavedDeviceInfo {
  host: string;
  username: string;
  hasPassword: boolean;
}

/** Generic device-memory seam; the store calls it best-effort. */
export interface DeviceMemoryGateway {
  /** Last remembered device, or null (first launch / failure). */
  load(): Promise<SavedDeviceInfo | null>;
  /** Persist identity + password after a successful probe. */
  save(input: { host: string; username: string; password: string }): Promise<void>;
  /** Delete remembered identity + password. */
  forget(): Promise<void>;
}

/**
 * Real gateway backed by Tauri commands. `load` never throws: outside Tauri
 * (plain browser dev) or on storage errors it degrades to null.
 */
export class TauriDeviceMemoryGateway implements DeviceMemoryGateway {
  async load(): Promise<SavedDeviceInfo | null> {
    try {
      return await invoke<SavedDeviceInfo | null>("get_remembered_device");
    } catch {
      return null;
    }
  }

  save(input: { host: string; username: string; password: string }): Promise<void> {
    return invoke("remember_device", {
      host: input.host,
      username: input.username,
      password: input.password,
    });
  }

  forget(): Promise<void> {
    return invoke("forget_remembered_device");
  }
}