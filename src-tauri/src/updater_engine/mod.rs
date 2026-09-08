//! Updater transaction engine (r2 §5.1–5.5).
//!
//! An install is a journaled transaction over a filesystem trait so every
//! fault matrix test runs against an in-memory sandbox. Invariants:
//! - Every phase change is persisted (state.json / journal.jsonl) BEFORE the
//!   irreversible operation it leads to.
//! - Recovery never leaves the machine without a complete bundle: `current`
//!   is the new healthy version or the previous known-good one.
//! - Health checks never involve the network (r2 §5.6).

pub mod fs_trait;

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::test_support::fault::FaultInjector;
use fs_trait::{FsAction, FsError, UpdateFilesystem};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Phase {
    Idle,
    Downloading,
    Downloaded,
    Verifying,
    Verified,
    Staging,
    BackingUp,
    Installing,
    PendingHealth,
    Committed,
    RecoveryRequired,
    RollingBack,
    RolledBack,
    FailedSafe,
}

/// Persistent state (r2 §5.4). Written before irreversible steps.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpdateState {
    pub schema: u32,
    pub transaction_id: String,
    pub from_version: String,
    pub to_version: String,
    pub phase: Phase,
    pub current_path: PathBuf,
    pub backup_path: Option<PathBuf>,
    pub staged_path: Option<PathBuf>,
    pub health_committed: bool,
}

#[derive(Debug)]
pub enum InstallError {
    Fault { point: String, message: String },
    Io { phase: Phase, message: String },
    Recovery { message: String },
}

impl std::fmt::Display for InstallError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            InstallError::Fault { point, message } => {
                write!(f, "injected fault at {point}: {message}")
            }
            InstallError::Io { phase, message } => write!(f, "{phase:?}: {message}"),
            InstallError::Recovery { message } => write!(f, "recovery failed: {message}"),
        }
    }
}

impl std::error::Error for InstallError {}

/// One update transaction. The state is what survives process death.
pub struct UpdaterTransaction {
    pub state: UpdateState,
}

impl UpdaterTransaction {
    pub fn new(
        transaction_id: &str,
        from_version: &str,
        to_version: &str,
        current_path: &Path,
    ) -> Self {
        Self {
            state: UpdateState {
                schema: 1,
                transaction_id: transaction_id.to_string(),
                from_version: from_version.to_string(),
                to_version: to_version.to_string(),
                phase: Phase::Idle,
                current_path: current_path.to_path_buf(),
                backup_path: None,
                staged_path: None,
                health_committed: false,
            },
        }
    }
}

/// Persist a phase change BEFORE the operation that follows it (r2 §5.4).
fn persist_phase(
    tx: &mut UpdaterTransaction,
    root: &Path,
    fs: &mut dyn UpdateFilesystem,
    phase: Phase,
) -> Result<(), InstallError> {
    tx.state.phase = phase;
    fs.append_journal(root, &tx.state.transaction_id, phase)
        .map_err(|e| InstallError::Io {
            phase,
            message: e.to_string(),
        })?;
    fs.write_state(root, &tx.state.transaction_id, &tx.state)
        .map_err(|e| InstallError::Io {
            phase,
            message: e.to_string(),
        })?;
    Ok(())
}

/// Fire an injected fault point; on trigger, mark RecoveryRequired (persisted)
/// and abort the transaction.
fn fire(
    tx: &mut UpdaterTransaction,
    root: &Path,
    fs: &mut dyn UpdateFilesystem,
    faults: &FaultInjector,
    point: &str,
) -> Result<(), InstallError> {
    match faults.trigger(point) {
        Ok(()) => Ok(()),
        Err(e) => {
            persist_phase(tx, root, fs, Phase::RecoveryRequired)?;
            Err(InstallError::Fault {
                point: point.to_string(),
                message: e.to_string(),
            })
        }
    }
}

/// Drive the install transaction until PendingHealth (the new bundle is in
/// place and awaiting its health checkpoints). Download/extract operate on
/// fake archives through the fs trait — the production adapter substitutes
/// the real network fetch and tar extraction (r2 layered architecture).
pub fn run_install(
    tx: &mut UpdaterTransaction,
    root: &Path,
    fs: &mut dyn UpdateFilesystem,
    faults: &FaultInjector,
) -> Result<(), InstallError> {
    let txn_dir = root.join("transactions").join(&tx.state.transaction_id);

    persist_phase(tx, root, fs, Phase::Idle)?;

    // ── Download ──────────────────────────────────────────────────────────
    fire(tx, root, fs, faults, "updater.download.before")?;
    persist_phase(tx, root, fs, Phase::Downloading)?;
    fire(tx, root, fs, faults, "updater.download.mid")?;
    let archive = txn_dir.join("update.tar.gz");
    fs.write_fake_archive(&archive)
        .map_err(|e| InstallError::Io {
            phase: tx.state.phase,
            message: e.to_string(),
        })?;
    fire(tx, root, fs, faults, "updater.download.after")?;
    persist_phase(tx, root, fs, Phase::Downloaded)?;

    // ── Verify ────────────────────────────────────────────────────────────
    persist_phase(tx, root, fs, Phase::Verifying)?;
    if !fs
        .verify_archive_hash(&archive)
        .map_err(|e| InstallError::Io {
            phase: tx.state.phase,
            message: e.to_string(),
        })?
    {
        persist_phase(tx, root, fs, Phase::RecoveryRequired)?;
        return Err(InstallError::Io {
            phase: Phase::Verifying,
            message: "archive hash mismatch (integrity check failed; release checkpoint)".into(),
        });
    }
    persist_phase(tx, root, fs, Phase::Verified)?;

    // ── Extract + stage ───────────────────────────────────────────────────
    fire(tx, root, fs, faults, "updater.extract.before")?;
    let extract = txn_dir.join("extract");
    fs.mkdir(&extract).map_err(|e| InstallError::Io {
        phase: tx.state.phase,
        message: e.to_string(),
    })?;
    fire(tx, root, fs, faults, "updater.extract.mid")?;
    let new_bundle = extract.join("Jetson Remote.app");
    fs.write_fake_archive_content(&new_bundle)
        .map_err(|e| InstallError::Io {
            phase: tx.state.phase,
            message: e.to_string(),
        })?;
    fire(tx, root, fs, faults, "updater.extract.after")?;

    persist_phase(tx, root, fs, Phase::Staging)?;
    let staged = root.join("staged").join(&tx.state.to_version);
    fs.record_action(FsAction::CopyBundle {
        from: new_bundle.clone(),
        to: staged.clone(),
    });
    fs.real_copy(&new_bundle, &staged)
        .map_err(|e| InstallError::Io {
            phase: tx.state.phase,
            message: e.to_string(),
        })?;
    tx.state.staged_path = Some(staged.clone());
    fs.write_state(root, &tx.state.transaction_id, &tx.state)
        .map_err(|e| InstallError::Io {
            phase: tx.state.phase,
            message: e.to_string(),
        })?;

    // ── Backup current ────────────────────────────────────────────────────
    fire(tx, root, fs, faults, "updater.backup.before")?;
    persist_phase(tx, root, fs, Phase::BackingUp)?;
    fire(tx, root, fs, faults, "updater.backup.mid")?;
    let backup = root.join("backup").join(&tx.state.from_version);
    if fs.exists(&backup) {
        fs.remove_dir(&backup).map_err(|e| InstallError::Io {
            phase: tx.state.phase,
            message: e.to_string(),
        })?;
    }
    fire(tx, root, fs, faults, "updater.backup.after")?;
    fs.record_action(FsAction::CopyBundle {
        from: tx.state.current_path.clone(),
        to: backup.clone(),
    });
    fs.real_copy(&tx.state.current_path.clone(), &backup)
        .map_err(|e| InstallError::Io {
            phase: tx.state.phase,
            message: e.to_string(),
        })?;
    tx.state.backup_path = Some(backup);
    fs.write_state(root, &tx.state.transaction_id, &tx.state)
        .map_err(|e| InstallError::Io {
            phase: tx.state.phase,
            message: e.to_string(),
        })?;

    // ── Swap ──────────────────────────────────────────────────────────────
    fire(tx, root, fs, faults, "updater.swap.before")?;
    persist_phase(tx, root, fs, Phase::Installing)?;
    fs.record_action(FsAction::RemoveDir {
        path: tx.state.current_path.clone(),
    });
    fs.remove_dir(&tx.state.current_path.clone())
        .map_err(|e| InstallError::Io {
            phase: tx.state.phase,
            message: e.to_string(),
        })?;
    fire(tx, root, fs, faults, "updater.swap.after")?;
    fs.record_action(FsAction::CopyBundle {
        from: staged.clone(),
        to: tx.state.current_path.clone(),
    });
    fs.real_copy(&staged, &tx.state.current_path.clone())
        .map_err(|e| InstallError::Io {
            phase: tx.state.phase,
            message: e.to_string(),
        })?;
    fs.write_state(root, &tx.state.transaction_id, &tx.state)
        .map_err(|e| InstallError::Io {
            phase: tx.state.phase,
            message: e.to_string(),
        })?;

    persist_phase(tx, root, fs, Phase::PendingHealth)?;

    // Launch is process-external (`open -n`); the NEW app run reads
    // PendingHealth and commits via commit_health() once its AppReady
    // checkpoints pass (r2 §5.7–5.8).
    Ok(())
}

/// Recovery entry: called at the next process start when a state.json
/// exists. Resolves the interrupted transaction to RolledBack (restore
/// known-good from backup) — or FailedSafe when only current exists.
pub fn run_recovery(
    root: &Path,
    fs: &mut dyn UpdateFilesystem,
    faults: &FaultInjector,
) -> Result<Phase, InstallError> {
    let state = match fs.read_state(root) {
        Ok(s) => s,
        Err(FsError::NotFound) => return Ok(Phase::Idle), // nothing to recover
        Err(e) => {
            return Err(InstallError::Recovery {
                message: e.to_string(),
            })
        }
    };

    match state.phase {
        Phase::Idle | Phase::Committed | Phase::RolledBack => return Ok(state.phase),
        Phase::FailedSafe => {
            // Already known-failed; caller decides. current must exist.
            if fs.exists(&state.current_path) {
                return Ok(Phase::FailedSafe);
            }
            return Err(InstallError::Recovery {
                message: "FailedSafe but current bundle missing".into(),
            });
        }
        _ => {}
    }

    // Simulate the deferred launch/commit decision point (fault matrix).
    if faults.trigger("updater.rollback.decide").is_err() {
        return Ok(state.phase);
    }

    // Interrupted somewhere mid-transaction → prefer rolling back to the
    // known-good backup.
    if let Some(backup) = state.backup_path.clone() {
        if fs.exists(&backup) {
            fs.append_journal(root, &state.transaction_id, Phase::RollingBack)
                .map_err(|e| InstallError::Recovery {
                    message: e.to_string(),
                })?;
            fs.record_action(FsAction::RemoveDir {
                path: state.current_path.clone(),
            });
            fs.remove_dir(&state.current_path)
                .map_err(|e| InstallError::Recovery {
                    message: e.to_string(),
                })?;
            fs.record_action(FsAction::CopyBundle {
                from: backup.clone(),
                to: state.current_path.clone(),
            });
            fs.real_copy(&backup, &state.current_path)
                .map_err(|e| InstallError::Recovery {
                    message: e.to_string(),
                })?;
            let mut final_state = state.clone();
            final_state.phase = Phase::RolledBack;
            final_state.health_committed = false;
            fs.write_state(root, &final_state.transaction_id, &final_state)
                .map_err(|e| InstallError::Recovery {
                    message: e.to_string(),
                })?;
            return Ok(Phase::RolledBack);
        }
    }

    // No backup available → FailedSafe only if the current bundle is whole.
    if fs.exists(&state.current_path) {
        let mut final_state = state.clone();
        final_state.phase = Phase::FailedSafe;
        fs.write_state(root, &final_state.transaction_id, &final_state)
            .map_err(|e| InstallError::Recovery {
                message: e.to_string(),
            })?;
        return Ok(Phase::FailedSafe);
    }
    Err(InstallError::Recovery {
        message: "no backup and current bundle missing".into(),
    })
}

/// Commit health after the NEW version passed its AppReady checkpoints.
/// Network checks must NOT gate health (r2 §5.6).
pub fn commit_health(
    root: &Path,
    fs: &mut dyn UpdateFilesystem,
    faults: &FaultInjector,
) -> Result<(), InstallError> {
    let mut state = fs.read_state(root).map_err(|e| InstallError::Recovery {
        message: e.to_string(),
    })?;
    if state.phase != Phase::PendingHealth {
        return Ok(());
    }
    faults
        .trigger("app.start.before_health_commit")
        .map_err(|e| InstallError::Fault {
            point: "app.start.before_health_commit".into(),
            message: e.to_string(),
        })?;
    state.health_committed = true;
    state.phase = Phase::Committed;
    fs.write_state(root, &state.transaction_id, &state)
        .map_err(|e| InstallError::Io {
            phase: state.phase,
            message: e.to_string(),
        })?;
    faults
        .trigger("app.start.after_health_commit")
        .map_err(|e| InstallError::Fault {
            point: "app.start.after_health_commit".into(),
            message: e.to_string(),
        })?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;
    use crate::test_support::fault::FaultAction;
    use fs_trait::MemFs;

    fn sandbox_fixture(fs: &mut MemFs) -> (PathBuf, PathBuf) {
        let root = PathBuf::from("/tmp/jr-updater-sandbox");
        let current = root.join("Applications/Jetson Remote.app");
        fs.write(&current.join("jrmarker"), b"old-known-good")
            .unwrap();
        (root, current)
    }

    fn fresh_tx(current: &Path, txn: &str) -> UpdaterTransaction {
        UpdaterTransaction::new(txn, "0.3.8", "0.4.0", current)
    }

    fn interrupted_state(root: &Path, phase: Phase, backup: Option<PathBuf>, fs: &mut MemFs) {
        let state = UpdateState {
            schema: 1,
            transaction_id: "01JUPDA".into(),
            from_version: "0.3.8".into(),
            to_version: "0.4.0".into(),
            phase,
            current_path: root.join("Applications/Jetson Remote.app"),
            backup_path: backup,
            staged_path: Some(root.join("staged/0.4.0")),
            health_committed: false,
        };
        fs.write_state(root, &state.transaction_id, &state).unwrap();
    }

    #[test]
    fn happy_path_reaches_pending_health_with_backup_recorded() {
        let mut fs = MemFs::new();
        let (root, current) = sandbox_fixture(&mut fs);
        let txn = "01JUPDA";
        let mut tx = fresh_tx(&current, txn);

        run_install(&mut tx, &root, &mut fs, &FaultInjector::new()).unwrap();

        assert_eq!(tx.state.phase, Phase::PendingHealth);
        assert!(!tx.state.health_committed);
        assert!(fs.exists(&root.join("backup/0.3.8/jrmarker")));
        // Swap step really replaced current with the staged new bundle.
        assert!(fs
            .read(&current.join("jrmarker"))
            .unwrap_or_default()
            .is_empty());
        let actions = fs.actions.clone();
        assert!(actions
            .iter()
            .any(|a| matches!(a, FsAction::RemoveDir { path } if path == &current)));
        assert!(actions
            .iter()
            .any(|a| matches!(a, FsAction::CopyBundle { from, to }
                if from == &root.join("staged/0.4.0") && to == &current)));
    }

    #[test]
    fn every_download_extract_backup_swap_fault_marks_recovery_required() {
        for point in [
            "updater.download.before",
            "updater.download.mid",
            "updater.download.after",
            "updater.extract.before",
            "updater.extract.mid",
            "updater.extract.after",
            "updater.backup.before",
            "updater.backup.mid",
            "updater.backup.after",
            "updater.swap.before",
            "updater.swap.after",
        ] {
            let mut fs = MemFs::new();
            let (root, current) = sandbox_fixture(&mut fs);
            let txn = "01JUPDA";
            let mut tx = fresh_tx(&current, txn);
            let mut faults = FaultInjector::new();
            faults.arm(point, FaultAction::Error("injected".into()));

            let res = run_install(&mut tx, &root, &mut fs, &faults);
            assert!(res.is_err(), "{point} must inject an error");
            assert_eq!(tx.state.phase, Phase::RecoveryRequired, "{point}");
        }
    }

    #[test]
    fn recovery_restores_backup_when_interrupted_after_backup() {
        let mut fs = MemFs::new();
        let (root, current) = sandbox_fixture(&mut fs);
        let backup = root.join("backup/0.3.8");
        // current already contains the new (broken) build; backup has old.
        fs.write(&backup.join("jrmarker"), b"old-known-good")
            .unwrap();
        fs.write(&current.join("jrmarker"), b"new-broken").unwrap();
        interrupted_state(&root, Phase::Installing, Some(backup), &mut fs);

        let phase = run_recovery(&root, &mut fs, &FaultInjector::new()).unwrap();
        assert_eq!(phase, Phase::RolledBack);
        assert_eq!(
            fs.read(&current.join("jrmarker")).unwrap(),
            b"old-known-good"
        );
    }

    #[test]
    fn recovery_from_pending_health_with_backup_rolls_back_known_good() {
        let mut fs = MemFs::new();
        let (root, current) = sandbox_fixture(&mut fs);
        let backup = root.join("backup/0.3.8");
        fs.write(&backup.join("jrmarker"), b"old-known-good")
            .unwrap();
        fs.write(&current.join("jrmarker"), b"new-untested")
            .unwrap();
        interrupted_state(&root, Phase::PendingHealth, Some(backup), &mut fs);

        let phase = run_recovery(&root, &mut fs, &FaultInjector::new()).unwrap();
        assert_eq!(phase, Phase::RolledBack);
        assert_eq!(
            fs.read(&current.join("jrmarker")).unwrap(),
            b"old-known-good"
        );
    }

    #[test]
    fn recovery_fails_safe_when_no_backup_but_current_exists() {
        let mut fs = MemFs::new();
        let (root, _) = sandbox_fixture(&mut fs);
        interrupted_state(&root, Phase::Installing, None, &mut fs);

        let phase = run_recovery(&root, &mut fs, &FaultInjector::new()).unwrap();
        assert_eq!(phase, Phase::FailedSafe);
    }

    #[test]
    fn recovery_is_idempotent_after_rolled_back() {
        let mut fs = MemFs::new();
        let (root, current) = sandbox_fixture(&mut fs);
        let backup = root.join("backup/0.3.8");
        fs.write(&backup.join("jrmarker"), b"old-known-good")
            .unwrap();
        fs.write(&current.join("jrmarker"), b"new-broken").unwrap();
        interrupted_state(&root, Phase::Installing, Some(backup), &mut fs);

        assert_eq!(
            run_recovery(&root, &mut fs, &FaultInjector::new()).unwrap(),
            Phase::RolledBack
        );
        // Second run sees RolledBack and returns it unchanged.
        assert_eq!(
            run_recovery(&root, &mut fs, &FaultInjector::new()).unwrap(),
            Phase::RolledBack
        );
    }

    #[test]
    fn commit_health_transitions_pending_to_committed() {
        let mut fs = MemFs::new();
        let (root, _) = sandbox_fixture(&mut fs);
        interrupted_state(&root, Phase::PendingHealth, None, &mut fs);

        commit_health(&root, &mut fs, &FaultInjector::new()).unwrap();
        let committed = fs.read_state(&root).unwrap();
        assert_eq!(committed.phase, Phase::Committed);
        assert!(committed.health_committed);
    }

    #[test]
    fn commit_health_fault_leaves_state_pending() {
        let mut fs = MemFs::new();
        let (root, _) = sandbox_fixture(&mut fs);
        interrupted_state(&root, Phase::PendingHealth, None, &mut fs);

        let mut faults = FaultInjector::new();
        faults.arm(
            "app.start.before_health_commit",
            FaultAction::Error("boom".into()),
        );
        assert!(commit_health(&root, &mut fs, &faults).is_err());
        assert_eq!(fs.read_state(&root).unwrap().phase, Phase::PendingHealth);
    }
}
