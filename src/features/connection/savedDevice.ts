// Remembered-device gateway (identity-v3): loads/saves/forgets the device
// memory through Tauri commands. The stored PASSWORD never comes back through
// this interface — `hasPassword` is a capability flag only (PRD §67).

import { invoke } from "@tauri-apps/api/core";
import { DevicePath } from "./types";

/** Shape of a remembered device as the frontend may know it. NO password. */
export interface SavedDeviceInfo {
  /** Stable identity (device-tree serial / machine-id); null for legacy v2 entries. */
  deviceId: string | null;
  username: string;
  /** The user-chosen display name; null until the device is named. */
  displayName: string | null;
  /** Current known candidate addresses (LAN / Tailscale). */
  paths: DevicePath[];
  /** The address the last successful connection used. */
  lastUsedPath: string | null;
  hasPassword: boolean;
}

/** Generic device-memory seam; the store calls it best-effort. */
export interface DeviceMemoryGateway {
  /** Every remembered device, most recently connected first ([] on failure). */
  loadAll(): Promise<SavedDeviceInfo[]>;
  /**
   * Persist one device's identity + paths + (typed) password after a
   * successful probe/naming. An empty password keeps the stored secret; the
   * backend merges legacy v2 `user@host` duplicates into the machine-id
   * identity.
   */
  save(input: {
    deviceId: string | null;
    username: string;
    displayName: string | null;
    paths: DevicePath[];
    entryHost: string;
    password: string;
  }): Promise<void>;
  /** Delete ONE remembered device's identity + password. */
  forget(input: {
    deviceId: string | null;
    host: string | null;
    username: string;
  }): Promise<void>;
}

/**
 * Real gateway backed by Tauri commands. `loadAll` never throws: outside
 * Tauri (plain browser dev) or on storage errors it degrades to an empty list.
 */
export class TauriDeviceMemoryGateway implements DeviceMemoryGateway {
  async loadAll(): Promise<SavedDeviceInfo[]> {
    try {
      const devices = await invoke<SavedDeviceInfo[]>("get_remembered_devices");
      return Array.isArray(devices) ? devices : [];
    } catch {
      return [];
    }
  }

  save(input: {
    deviceId: string | null;
    username: string;
    displayName: string | null;
    paths: DevicePath[];
    entryHost: string;
    password: string;
  }): Promise<void> {
    return invoke("remember_device", {
      input: {
        deviceId: input.deviceId,
        username: input.username,
        displayName: input.displayName,
        paths: input.paths,
        entryHost: input.entryHost,
        password: input.password,
      },
    });
  }

  forget(input: {
    deviceId: string | null;
    host: string | null;
    username: string;
  }): Promise<void> {
    return invoke("forget_remembered_device", {
      deviceId: input.deviceId,
      host: input.host,
      username: input.username,
    });
  }
}
