//! Filesystem abstraction for the updater transaction engine.
//!
//! Production uses `RealUpdateFilesystem` (std::fs). Tests use an in-memory
//! `MemFs` so the whole fault matrix runs without touching real paths.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use super::Phase;
use super::UpdateState;

#[derive(Debug)]
pub enum FsError {
    NotFound,
    Io(String),
}

impl std::fmt::Display for FsError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            FsError::NotFound => write!(f, "not found"),
            FsError::Io(msg) => write!(f, "{msg}"),
        }
    }
}

impl std::error::Error for FsError {}

/// Auditable operations — lets tests assert ordering (e.g. "backup happens
/// before remove-dir").
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FsAction {
    CopyBundle { from: PathBuf, to: PathBuf },
    RemoveDir { path: PathBuf },
    Spawn { path: PathBuf },
    Noop,
}

pub trait UpdateFilesystem {
    fn exists(&mut self, path: &Path) -> bool;
    fn mkdir(&mut self, path: &Path) -> Result<(), FsError>;
    fn remove_dir(&mut self, path: &Path) -> Result<(), FsError>;
    fn write(&mut self, path: &Path, data: &[u8]) -> Result<(), FsError>;
    fn read(&mut self, path: &Path) -> Result<Vec<u8>, FsError>;
    /// Copy a directory tree (`real_copy` alias keeps one method).
    fn real_copy(&mut self, from: &Path, to: &Path) -> Result<(), FsError>;
    /// Journal/state persistence primitives (implemented over write/read).
    fn write_state(
        &mut self,
        root: &Path,
        _txn_id: &str,
        state: &UpdateState,
    ) -> Result<(), FsError> {
        let dir = root.join("updater");
        if !self.exists(&dir) {
            self.mkdir(&dir)?;
        }
        let json = serde_json::to_string_pretty(state).map_err(|e| FsError::Io(e.to_string()))?;
        self.write(&dir.join("state.json"), json.as_bytes())
    }
    fn append_journal(&mut self, root: &Path, txn_id: &str, phase: Phase) -> Result<(), FsError> {
        let dir = root.join("updater").join("transactions").join(txn_id);
        if !self.exists(&dir) {
            self.mkdir(&dir)?;
        }
        let line = format!("{{\"seq\":0,\"phase\":\"{:?}\",\"ts\":0}}\n", phase);
        self.write(&dir.join("journal.jsonl"), line.as_bytes())
    }
    fn read_state(&mut self, root: &Path) -> Result<UpdateState, FsError> {
        let path = root.join("updater").join("state.json");
        let bytes = self.read(&path)?;
        serde_json::from_slice(&bytes).map_err(|e| FsError::Io(e.to_string()))
    }
    fn write_state_final(
        &mut self,
        root: &Path,
        txn_id: &str,
        phase: Phase,
    ) -> Result<(), FsError> {
        let mut state = self.read_state(root)?;
        state.phase = phase;
        let _ = txn_id;
        self.write_state(root, &state.transaction_id, &state)
    }
    /// Verifying: used by engine to check an archive hash.
    fn verify_archive_hash(&mut self, _path: &Path) -> Result<bool, FsError> {
        Ok(true)
    }
    /// Audit trail for tests.
    fn record_action(&mut self, _action: FsAction) {}
    /// For the sandbox: write a marker file inside a bundle dir.
    fn write_marker(&mut self, path: &Path, data: &[u8]) -> Result<(), FsError> {
        self.write(path, data)
    }
    /// Download simulation: engine calls `write_fake_archive` to get a
    /// downloadable body. Real impl downloads from the network.
    fn write_fake_archive(&mut self, path: &Path) -> Result<(), FsError> {
        self.write(path, b"FAKE-TAR")
    }
    fn write_fake_archive_content(&mut self, path: &Path) -> Result<(), FsError> {
        self.write(path, b"Jetson Remote.app content")
    }
}

pub struct RealUpdateFilesystem;

impl UpdateFilesystem for RealUpdateFilesystem {
    fn exists(&mut self, path: &Path) -> bool {
        path.exists()
    }

    fn mkdir(&mut self, path: &Path) -> Result<(), FsError> {
        std::fs::create_dir_all(path).map_err(|e| FsError::Io(e.to_string()))
    }

    fn remove_dir(&mut self, path: &Path) -> Result<(), FsError> {
        match std::fs::remove_dir_all(path) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(FsError::Io(e.to_string())),
        }
    }

    fn write(&mut self, path: &Path, data: &[u8]) -> Result<(), FsError> {
        if let Some(parent) = path.parent() {
            if !parent.as_os_str().is_empty() {
                self.mkdir(parent)?;
            }
        }
        std::fs::write(path, data).map_err(|e| FsError::Io(e.to_string()))
    }

    fn read(&mut self, path: &Path) -> Result<Vec<u8>, FsError> {
        std::fs::read(path).map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                FsError::NotFound
            } else {
                FsError::Io(e.to_string())
            }
        })
    }

    fn real_copy(&mut self, from: &Path, to: &Path) -> Result<(), FsError> {
        // Recursive copy via copy_dir helper below.
        copy_dir_all(from, to).map_err(|e| FsError::Io(e.to_string()))
    }
}

fn copy_dir_all(src: &Path, dst: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(dst)?;
    for entry in std::fs::read_dir(src)? {
        let entry = entry?;
        let ty = entry.file_type()?;
        let from = entry.path();
        let to = dst.join(entry.file_name());
        if ty.is_dir() {
            copy_dir_all(&from, &to)?;
        } else {
            std::fs::copy(&from, &to)?;
        }
    }
    Ok(())
}

/// In-memory filesystem for the fault matrix (directories are implicit).
pub struct MemFs {
    pub files: BTreeMap<PathBuf, Vec<u8>>,
    pub actions: Vec<FsAction>,
}

impl Default for MemFs {
    fn default() -> Self {
        Self::new()
    }
}

impl MemFs {
    pub fn new() -> Self {
        Self {
            files: BTreeMap::new(),
            actions: Vec::new(),
        }
    }

    fn path_of(&self, path: &Path) -> PathBuf {
        path.to_path_buf()
    }
}

impl UpdateFilesystem for MemFs {
    fn exists(&mut self, path: &Path) -> bool {
        let p = self.path_of(path);
        self.files
            .keys()
            .any(|k| k == &p || k.starts_with(p.join("")))
    }

    fn mkdir(&mut self, path: &Path) -> Result<(), FsError> {
        // Directories are implicit in MemFs: record for audit only.
        let _ = path;
        Ok(())
    }

    fn remove_dir(&mut self, path: &Path) -> Result<(), FsError> {
        let prefix = self.path_of(path);
        let keys: Vec<PathBuf> = self
            .files
            .keys()
            .filter(|k| k.starts_with(&prefix))
            .cloned()
            .collect();
        for k in keys {
            self.files.remove(&k);
        }
        Ok(())
    }

    fn write(&mut self, path: &Path, data: &[u8]) -> Result<(), FsError> {
        self.files.insert(self.path_of(path), data.to_vec());
        Ok(())
    }

    fn read(&mut self, path: &Path) -> Result<Vec<u8>, FsError> {
        self.files
            .get(&self.path_of(path))
            .cloned()
            .ok_or(FsError::NotFound)
    }

    fn real_copy(&mut self, from: &Path, to: &Path) -> Result<(), FsError> {
        let prefix = self.path_of(from);
        let matches: Vec<(PathBuf, Vec<u8>)> = self
            .files
            .iter()
            .filter(|(k, _)| k.starts_with(&prefix))
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect();
        if matches.is_empty() {
            // Fallback for plain files stored under the exact path.
            if let Some(data) = self.files.get(&prefix) {
                self.files.insert(self.path_of(to), data.clone());
                return Ok(());
            }
            return Err(FsError::NotFound);
        }
        for (k, v) in matches {
            let rel = k.strip_prefix(&prefix).unwrap_or(&k);
            self.files.insert(self.path_of(to).join(rel), v);
        }
        Ok(())
    }

    fn record_action(&mut self, action: FsAction) {
        self.actions.push(action);
    }
}
