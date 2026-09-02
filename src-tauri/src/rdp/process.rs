use std::collections::VecDeque;
use std::process::Stdio;
use std::sync::{Arc, Mutex};

use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStderr, ChildStdout, Command};

use super::args::{stdin_payload, FreeRdpArguments};
use super::error::RdpError;
use super::types::RdpStatus;

/// Max retained stdout/stderr lines per process (diagnostics ring buffer).
pub const MAX_LOG_LINES: usize = 300;

/// Grace period between SIGTERM and SIGKILL when closing (ms).
const CLOSE_GRACE_MILLIS: u64 = 2000;

/// A live FreeRDP sidecar process. Owns the child handle (for status/close)
/// and a ring buffer of recent stdout/stderr lines — drained here so the
/// child's pipes never back up, never persisted, never containing the stdin
/// credential (the password only ever goes *into* stdin, not out of stdout).
pub struct RdpProcess {
    child: Child,
    logs: Arc<Mutex<VecDeque<String>>>,
}

impl RdpProcess {
    /// Adopt an already-spawned child, draining its stdout/stderr (if piped)
    /// into the diagnostics ring buffer.
    pub(crate) fn from_child(mut child: Child) -> Self {
        let stdout = child.stdout.take();
        let stderr = child.stderr.take();
        let logs = Arc::new(Mutex::new(VecDeque::new()));
        if stdout.is_some() || stderr.is_some() {
            let sink = logs.clone();
            tokio::spawn(async move {
                drain_streams(stdout, stderr, sink).await;
            });
        }
        Self { child, logs }
    }

    pub fn status(&mut self) -> RdpStatus {
        match self.child.try_wait() {
            Ok(Some(status)) => RdpStatus::Exited {
                exit_code: status.code(),
                error: None,
            },
            Ok(None) => RdpStatus::Running,
            Err(_) => RdpStatus::Exited {
                exit_code: None,
                error: None,
            },
        }
    }

    pub fn is_running(&mut self) -> bool {
        matches!(self.status(), RdpStatus::Running)
    }

    /// Graceful close: SIGTERM, a bounded grace period, then SIGKILL. FreeRDP
    /// handles SIGTERM as a clean session teardown (leaving the remote Xorg
    /// session alive — PRD §24), so the force-kill is a rare fallback.
    pub async fn close(&mut self) -> Result<(), RdpError> {
        if let Some(pid) = self.child.id() {
            // SAFETY: kill(2) with a valid pid and a standard signal.
            unsafe {
                libc::kill(pid as libc::c_int, libc::SIGTERM);
            }
            tokio::time::sleep(std::time::Duration::from_millis(CLOSE_GRACE_MILLIS)).await;
        }
        if self.is_running() {
            let _ = self.child.start_kill();
        }
        let _ = self.child.wait().await;
        Ok(())
    }

    /// Immediate SIGKILL, used by app-exit cleanup (Drop) where we cannot await.
    pub fn kill_now(&mut self) {
        let _ = self.child.start_kill();
    }

    /// Recent stdout/stderr lines for future diagnostics. Capped at
    /// `MAX_LOG_LINES`, memory-only.
    #[allow(dead_code)] // diagnostics (Phase 6)
    pub fn logs_snapshot(&self) -> Vec<String> {
        self.logs.lock().unwrap().iter().cloned().collect()
    }
}

/// Spawn a FreeRDP sidecar: build the process from `argv`, write the full
/// argument set (including the password) to stdin, close stdin, and attach log
/// draining. The password never appears in `argv`.
pub async fn spawn_sidecar(args: &FreeRdpArguments) -> Result<RdpProcess, RdpError> {
    if args.argv.is_empty() {
        return Err(RdpError::Unknown);
    }
    let mut cmd = Command::new(&args.argv[0]);
    cmd.args(&args.argv[1..])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = cmd.spawn().map_err(RdpError::LaunchFailed)?;

    let mut stdin = child
        .stdin
        .take()
        .ok_or_else(|| RdpError::LaunchFailed(std::io::Error::other("stdin unavailable")))?;
    let payload = stdin_payload(args);
    if let Err(e) = stdin.write_all(&payload).await {
        let _ = child.start_kill();
        return Err(RdpError::LaunchFailed(e));
    }
    drop(stdin); // close stdin → FreeRDP stops waiting for more args

    Ok(RdpProcess::from_child(child))
}

async fn drain_streams(
    stdout: Option<ChildStdout>,
    stderr: Option<ChildStderr>,
    logs: Arc<Mutex<VecDeque<String>>>,
) {
    let out_logs = logs.clone();
    let err_logs = logs;
    let out = async move {
        if let Some(reader) = stdout {
            drain_reader(reader, out_logs).await;
        }
    };
    let err = async move {
        if let Some(reader) = stderr {
            drain_reader(reader, err_logs).await;
        }
    };
    tokio::join!(out, err);
}

async fn drain_reader<R>(reader: R, logs: Arc<Mutex<VecDeque<String>>>)
where
    R: tokio::io::AsyncRead + Unpin,
{
    let mut lines = BufReader::new(reader).lines();
    while let Ok(Some(line)) = lines.next_line().await {
        push_log(&logs, line);
    }
}

fn push_log(logs: &Mutex<VecDeque<String>>, line: String) {
    if line.trim().is_empty() {
        return;
    }
    let mut guard = logs.lock().unwrap();
    if guard.len() >= MAX_LOG_LINES {
        guard.pop_front();
    }
    guard.push_back(line);
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn sleep_child(secs: u64) -> Child {
        Command::new("/bin/sleep")
            .arg(secs.to_string())
            .spawn()
            .unwrap()
    }

    #[tokio::test]
    async fn status_running_then_exited_after_kill() {
        let child = sleep_child(30).await;
        let mut p = RdpProcess::from_child(child);
        assert!(p.is_running());
        assert_eq!(p.status(), RdpStatus::Running);

        p.kill_now();
        let _ = p.child.wait().await;
        assert!(matches!(p.status(), RdpStatus::Exited { .. }));
        assert!(!p.is_running());
    }

    #[tokio::test]
    async fn close_reaps_a_running_sleep() {
        let child = sleep_child(30).await;
        let mut p = RdpProcess::from_child(child);
        p.close().await.unwrap();
        assert!(matches!(p.status(), RdpStatus::Exited { .. }));
    }

    #[tokio::test]
    async fn from_child_without_pipes_does_not_spawn_drain() {
        let child = sleep_child(30).await;
        let p = RdpProcess::from_child(child);
        // no piped stdout/stderr → no drain task; still owns the child fine.
        assert!(p.logs.lock().unwrap().is_empty());
    }
}
