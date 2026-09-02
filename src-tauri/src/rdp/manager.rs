use std::sync::Mutex;

use super::error::RdpError;
use super::process::RdpProcess;
use super::types::RdpStatus;

/// Manages the single active RDP sidecar (PRD §16: one desktop per device —
/// and MVP is single-device). Held as `tauri::State`; its `Drop` kills the
/// child so app exit never orphans an `sdl-freerdp` process (PRD §23).
pub struct RdpProcessManager {
    active: Mutex<Option<RdpProcess>>,
}

impl RdpProcessManager {
    pub fn new() -> Self {
        Self {
            active: Mutex::new(None),
        }
    }

    pub fn is_running(&self) -> bool {
        let mut guard = self.active.lock().unwrap();
        match guard.as_mut() {
            Some(p) => p.is_running(),
            None => false,
        }
    }

    pub fn status(&self) -> RdpStatus {
        let mut guard = self.active.lock().unwrap();
        match guard.as_mut() {
            Some(p) => p.status(),
            None => RdpStatus::NotRunning,
        }
    }

    /// Store a newly-launched process. Caller must have already checked
    /// `is_running()` to avoid clobbering a live session.
    pub fn set(&self, process: RdpProcess) {
        *self.active.lock().unwrap() = Some(process);
    }

    /// Gracefully close the active desktop, releasing the slot. Idempotent.
    pub async fn close(&self) -> Result<(), RdpError> {
        // Take the process out of the slot first so we don't hold the mutex
        // across the graceful-close awaits (a std::sync::MutexGuard is !Send).
        let process = self.active.lock().unwrap().take();
        if let Some(mut p) = process {
            p.close().await?;
        }
        Ok(())
    }

    /// Clear the slot without closing (used when a launched process has
    /// already exited; keeps the next launch clean).
    pub fn clear_if_exited(&self) {
        let mut guard = self.active.lock().unwrap();
        if let Some(p) = guard.as_mut() {
            if !p.is_running() {
                *guard = None;
            }
        }
    }
}

impl Default for RdpProcessManager {
    fn default() -> Self {
        Self::new()
    }
}

impl Drop for RdpProcessManager {
    fn drop(&mut self) {
        // Best-effort: SIGKILL any child still alive at app shutdown.
        if let Ok(mut guard) = self.active.lock() {
            if let Some(p) = guard.as_mut() {
                p.kill_now();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::process::Command;

    async fn running_manager() -> RdpProcessManager {
        let mgr = RdpProcessManager::new();
        let child = Command::new("/bin/sleep").arg("30").spawn().unwrap();
        mgr.set(RdpProcess::from_child(child));
        mgr
    }

    #[tokio::test]
    async fn reports_running_then_not_running() {
        let mgr = running_manager().await;
        assert!(mgr.is_running());
        assert_eq!(mgr.status(), RdpStatus::Running);

        mgr.close().await.unwrap();
        assert!(!mgr.is_running());
        assert_eq!(mgr.status(), RdpStatus::NotRunning);
    }

    #[tokio::test]
    async fn close_without_active_is_idempotent() {
        let mgr = RdpProcessManager::new();
        mgr.close().await.unwrap();
        assert_eq!(mgr.status(), RdpStatus::NotRunning);
    }

    #[test]
    fn default_status_is_not_running() {
        assert_eq!(RdpProcessManager::new().status(), RdpStatus::NotRunning);
    }

    #[tokio::test]
    async fn clear_if_exited_releases_a_dead_process() {
        let mgr = RdpProcessManager::new();
        // /bin/sleep 0 exits immediately, leaving a dead child in the slot.
        let child = Command::new("/bin/sleep").arg("0").spawn().unwrap();
        mgr.set(RdpProcess::from_child(child));
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;

        assert!(!mgr.is_running());
        assert!(matches!(mgr.status(), RdpStatus::Exited { .. }));

        mgr.clear_if_exited();
        assert_eq!(mgr.status(), RdpStatus::NotRunning);
    }
}
