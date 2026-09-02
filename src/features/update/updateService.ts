import { invoke } from "@tauri-apps/api/core";

export interface UpdateCheckResult {
  currentVersion: string;
  latestVersion: string | null;
  updateAvailable: boolean;
  releaseUrl: string | null;
  appAssetUrl: string | null;
  isBundledApp: boolean;
}

export interface UpdateError {
  code: string;
  message: string;
}

export function checkForUpdate(): Promise<UpdateCheckResult> {
  return invoke<UpdateCheckResult>("check_for_update");
}

export function downloadAndInstallUpdate(url: string): Promise<void> {
  return invoke<void>("download_and_install_update", { url });
}

export function isUpdateError(err: unknown): err is UpdateError {
  return (
    typeof err === "object" &&
    err !== null &&
    "message" in err &&
    "code" in err
  );
}