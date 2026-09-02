//! In-app loopback SSH tunnel (KI-004 / KI-021).
//!
//! macOS Local Network privacy (TCC) silently blocks LAN sockets for
//! unsigned/ad-hoc binaries, and the previous release workaround required the
//! user to keep a manual `ssh -L 2222:localhost:22 -L 3389:localhost:3389`
//! terminal tunnel alive. Neither is acceptable UX, so the app now spawns the
//! *system* `/usr/bin/ssh` (Apple-signed, therefore exempt from the TCC
//! block) to carry both planes over 127.0.0.1:
//!
//! - SSH control plane (russh)  → `127.0.0.1:<ssh_port>`
//! - RDP plane (embedded FreeRDP) → `127.0.0.1:<rdp_port>`
//!
//! The password reaches `ssh` only through an `SSH_ASKPASS` helper script in a
//! 0700 directory (never argv); the directory is removed when the tunnel dies.
//! The tunnel lives for the app process lifetime and is killed on exit.

use std::collections::HashMap;
use std::fs;
use std::io::Read as _;
use std::net::{TcpListener, TcpStream};
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde::Serialize;
use tauri::{AppHandle, Manager};

/// Preferred local ports — stable across runs so the host-key trust store
/// (keyed by wire host:port) doesn't churn. Falls back to ephemeral ports
/// when something else already listens on them.
pub const PREFERRED_SSH_PORT: u16 = 2222;
pub const PREFERRED_RDP_PORT: u16 = 3389;

const SSH_BIN: &str = "/usr/bin/ssh";
const READY_TIMEOUT: Duration = Duration::from_secs(25);
/// After the local forwards accept connections, give `ssh` this long to
/// finish (or fail) authentication before we declare the tunnel healthy.
const AUTH_GRACE: Duration = Duration::from_secs(3);

/// Loopback endpoints the planes must use instead of the LAN host.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct TunnelEndpoints {
    pub host: String,
    pub ssh_port: u16,
    pub rdp_port: u16,
}

#[derive(Debug, thiserror::Error)]
pub enum TunnelError {
    #[error("could not reach the device")]
    Unreachable,
    #[error("authentication failed")]
    AuthFailed,
    #[error("tunnel setup failed: {0}")]
    Setup(String),
}

struct ActiveTunnel {
    child: Child,
    endpoints: TunnelEndpoints,
    /// 0700 dir holding the askpass helper + password file; removed on drop.
    dir: PathBuf,
}

impl Drop for ActiveTunnel {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        let _ = fs::remove_dir_all(&self.dir);
    }
}

/// One tunnel per remote device (multi-device support, V0.4), keyed by
/// `username@host:remote_ssh_port`. Each tunnel gets its own loopback ports
/// (preferred ports for the first one, ephemeral for the rest), so several
/// Jetsons can be connected simultaneously. Tunnels live for the app process
/// lifetime and are all killed on exit.
#[derive(Clone, Default)]
pub struct TunnelManager {
    inner: Arc<Mutex<HashMap<String, ActiveTunnel>>>,
}

fn tunnel_key(host: &str, remote_ssh_port: u16, username: &str) -> String {
    format!("{username}@{host}:{remote_ssh_port}")
}

impl TunnelManager {
    pub fn new() -> Self {
        sweep_orphans();
        Self::default()
    }

    /// Terminate every tunnel (app exit). Idempotent.
    pub fn close_all(&self) {
        let mut guard = self.inner.lock().unwrap();
        guard.clear(); // Drop kills each child and removes its secret files.
    }

    /// Ensure a tunnel to `host` exists and return the loopback endpoints.
    /// Blocking (spawns ssh + polls ports); call from `spawn_blocking`.
    pub fn ensure(
        &self,
        app: &AppHandle,
        host: &str,
        remote_ssh_port: u16,
        username: &str,
        password: &str,
    ) -> Result<TunnelEndpoints, TunnelError> {
        // Dev override: a manually managed external tunnel (compile-time env,
        // see CONNECTION_REGRESSION_GUIDE §2.4) skips the in-app tunnel.
        if let Some((ssh, rdp)) = external_tunnel_ports() {
            return Ok(TunnelEndpoints {
                host: "127.0.0.1".into(),
                ssh_port: ssh,
                rdp_port: rdp,
            });
        }

        let key = tunnel_key(host, remote_ssh_port, username);
        let mut guard = self.inner.lock().unwrap();
        if let Some(t) = guard.get_mut(&key) {
            if is_healthy(t) {
                eprintln!(
                    "[jr-flow] tunnel reuse {}:{} / {}:{}",
                    t.endpoints.host, t.endpoints.ssh_port, t.endpoints.host, t.endpoints.rdp_port
                );
                return Ok(t.endpoints.clone());
            }
            // Stale tunnel for this device: drop kills the child + files;
            // tunnels to OTHER devices are untouched (multi-device, V0.4).
            guard.remove(&key);
        }

        eprintln!("[jr-flow] tunnel spawn host={host} user={username}");
        let tunnel = spawn_tunnel(app, host, remote_ssh_port, username, password)?;
        let endpoints = tunnel.endpoints.clone();
        guard.insert(key, tunnel);
        Ok(endpoints)
    }
}

/// Kill orphaned tunnel `ssh -N` processes left behind by a previous app
/// instance that was force-killed or crashed (normal exit reaps them via
/// RunEvent::Exit + Drop). Orphans hold the preferred local ports, which
/// would force ephemeral ports and churn the host-key trust store. The
/// match is safe: only our tunnel ssh carries the app-scoped known_hosts
/// path in its command line.
fn sweep_orphans() {
    let Ok(home) = std::env::var("HOME") else {
        return;
    };
    let known =
        format!("{home}/Library/Application Support/com.jetsonremote.app/tunnel/known_hosts");
    let Ok(out) = Command::new("/usr/bin/pgrep")
        .arg("-f")
        .arg(&known)
        .output()
    else {
        return;
    };
    if !out.status.success() {
        return;
    }
    for line in String::from_utf8_lossy(&out.stdout).lines() {
        if let Ok(pid) = line.trim().parse::<libc::pid_t>() {
            eprintln!("[jr-flow] tunnel sweep orphan pid={pid}");
            unsafe { libc::kill(pid, libc::SIGTERM) };
        }
    }
}

/// Compile-time dev override: `VITE_JR_SSH_PORT=2222 cargo tauri dev` rides an
/// externally managed tunnel exactly like the pre-0.2.1 frontend routing did.
fn external_tunnel_ports() -> Option<(u16, u16)> {
    let ssh: u16 = option_env!("VITE_JR_SSH_PORT")?.parse().ok()?;
    let rdp = option_env!("VITE_JR_RDP_PORT")
        .and_then(|v| v.parse().ok())
        .unwrap_or(PREFERRED_RDP_PORT);
    Some((ssh, rdp))
}

fn is_healthy(t: &mut ActiveTunnel) -> bool {
    // Child still running...
    match t.child.try_wait() {
        Ok(None) => {}
        _ => return false,
    }
    // ...and both forwards still accept connections.
    port_open(t.endpoints.ssh_port) && port_open(t.endpoints.rdp_port)
}

fn port_open(port: u16) -> bool {
    TcpStream::connect_timeout(
        &std::net::SocketAddr::from(([127, 0, 0, 1], port)),
        Duration::from_millis(250),
    )
    .is_ok()
}

fn port_free(port: u16) -> bool {
    TcpListener::bind(std::net::SocketAddr::from(([127, 0, 0, 1], port))).is_ok()
}

fn free_port() -> Result<u16, TunnelError> {
    TcpListener::bind(std::net::SocketAddr::from(([127, 0, 0, 1], 0)))
        .map(|l| l.local_addr().map(|a| a.port()).unwrap_or(0))
        .map_err(|e| TunnelError::Setup(format!("no free local port: {e}")))
        .and_then(|p| {
            if p == 0 {
                Err(TunnelError::Setup("no free local port".into()))
            } else {
                Ok(p)
            }
        })
}

fn pick_ports() -> Result<(u16, u16), TunnelError> {
    if port_free(PREFERRED_SSH_PORT) && port_free(PREFERRED_RDP_PORT) {
        return Ok((PREFERRED_SSH_PORT, PREFERRED_RDP_PORT));
    }
    Ok((free_port()?, free_port()?))
}

#[cfg(unix)]
fn chmod(path: &std::path::Path, mode: u32) -> Result<(), TunnelError> {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(path, fs::Permissions::from_mode(mode))
        .map_err(|e| TunnelError::Setup(format!("chmod {}: {e}", path.display())))
}

fn spawn_tunnel(
    app: &AppHandle,
    host: &str,
    remote_ssh_port: u16,
    username: &str,
    password: &str,
) -> Result<ActiveTunnel, TunnelError> {
    let base = app
        .path()
        .app_config_dir()
        .map_err(|e| TunnelError::Setup(format!("config dir: {e}")))?;
    let dir = base.join("tunnel");
    fs::create_dir_all(&dir).map_err(|e| TunnelError::Setup(format!("mkdir: {e}")))?;
    chmod(&dir, 0o700)?;

    // Password reaches ssh only via the askpass helper (never argv).
    let pw_file = dir.join("pw");
    fs::write(&pw_file, format!("{password}\n"))
        .map_err(|e| TunnelError::Setup(format!("write pw: {e}")))?;
    chmod(&pw_file, 0o600)?;
    let askpass = dir.join("askpass.sh");
    fs::write(
        &askpass,
        format!("#!/bin/sh\ncat \"{}\"\n", pw_file.display()),
    )
    .map_err(|e| TunnelError::Setup(format!("write askpass: {e}")))?;
    chmod(&askpass, 0o700)?;
    let known_hosts = dir.join("known_hosts");

    let (ssh_port, rdp_port) = pick_ports()?;

    let mut cmd = Command::new(SSH_BIN);
    cmd.arg("-N")
        .arg("-T")
        .arg("-F")
        .arg("/dev/null") // ignore the user's ssh config: deterministic tunnel
        .arg("-p")
        .arg(remote_ssh_port.to_string())
        .arg("-L")
        .arg(format!("127.0.0.1:{ssh_port}:localhost:22"))
        .arg("-L")
        .arg(format!("127.0.0.1:{rdp_port}:localhost:3389"))
        .arg("-o")
        .arg("ExitOnForwardFailure=yes")
        .arg("-o")
        .arg("StrictHostKeyChecking=accept-new")
        .arg("-o")
        .arg(format!("UserKnownHostsFile={}", known_hosts.display()))
        .arg("-o")
        .arg("NumberOfPasswordPrompts=1")
        .arg("-o")
        .arg("PreferredAuthentications=password,keyboard-interactive")
        .arg("-o")
        .arg("PubkeyAuthentication=no")
        .arg("-o")
        .arg("ServerAliveInterval=15")
        .arg("-o")
        .arg("ServerAliveCountMax=3")
        .arg(format!("{username}@{host}"))
        .env("SSH_ASKPASS", &askpass)
        .env("SSH_ASKPASS_REQUIRE", "force")
        .env("DISPLAY", ":0")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped());

    let mut child = cmd
        .spawn()
        .map_err(|e| TunnelError::Setup(format!("spawn {SSH_BIN}: {e}")))?;

    // Local -L listeners open before auth completes, so "ports open" alone is
    // not success: after they open, wait AUTH_GRACE for ssh to die on an auth
    // failure; surviving the grace means authentication went through.
    let deadline = Instant::now() + READY_TIMEOUT;
    loop {
        if let Some(status) = child
            .try_wait()
            .map_err(|e| TunnelError::Setup(format!("wait: {e}")))?
        {
            let stderr = drain_stderr(&mut child);
            return Err(classify_exit(&stderr, status.code()));
        }
        if port_open(ssh_port) && port_open(rdp_port) {
            let grace_end = Instant::now() + AUTH_GRACE;
            let mut failed = None;
            while Instant::now() < grace_end {
                std::thread::sleep(Duration::from_millis(200));
                if let Ok(Some(status)) = child.try_wait() {
                    let stderr = drain_stderr(&mut child);
                    failed = Some(classify_exit(&stderr, status.code()));
                    break;
                }
            }
            match failed {
                Some(e) => return Err(e),
                None => {
                    eprintln!("[jr-flow] tunnel up 127.0.0.1:{ssh_port} / 127.0.0.1:{rdp_port}");
                    return Ok(ActiveTunnel {
                        child,
                        endpoints: TunnelEndpoints {
                            host: "127.0.0.1".into(),
                            ssh_port,
                            rdp_port,
                        },
                        dir,
                    });
                }
            }
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            return Err(TunnelError::Unreachable);
        }
        std::thread::sleep(Duration::from_millis(200));
    }
}

fn drain_stderr(child: &mut Child) -> String {
    let mut buf = String::new();
    if let Some(mut err) = child.stderr.take() {
        let _ = err.read_to_string(&mut buf);
    }
    buf
}

/// Map an ssh exit into a product-meaningful error.
fn classify_exit(stderr: &str, code: Option<i32>) -> TunnelError {
    let low = stderr.to_lowercase();
    if low.contains("permission denied") || low.contains("authentication failed") {
        return TunnelError::AuthFailed;
    }
    if low.contains("no route to host")
        || low.contains("network is unreachable")
        || low.contains("connection refused")
        || low.contains("could not resolve hostname")
        || low.contains("operation timed out")
        || low.contains("connection timed out")
        || low.contains("unreachable")
    {
        return TunnelError::Unreachable;
    }
    let first = stderr.lines().find(|l| !l.trim().is_empty()).unwrap_or("");
    TunnelError::Setup(format!(
        "ssh exited {}{}",
        code.map(|c| c.to_string()).unwrap_or_else(|| "?".into()),
        if first.is_empty() {
            String::new()
        } else {
            format!(": {first}")
        }
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classify_exit_maps_auth_and_network() {
        assert!(matches!(
            classify_exit(
                "seeed@192.168.100.164: Permission denied (publickey,password).",
                Some(255)
            ),
            TunnelError::AuthFailed
        ));
        assert!(matches!(
            classify_exit(
                "ssh: connect to host 10.0.0.9 port 22: No route to host",
                Some(255)
            ),
            TunnelError::Unreachable
        ));
        match classify_exit("bind [127.0.0.1]:2222: Address already in use", Some(255)) {
            TunnelError::Setup(m) => assert!(m.contains("Address already in use")),
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn pick_ports_returns_free_ports() {
        let (a, b) = pick_ports().unwrap();
        assert_ne!(a, 0);
        assert_ne!(b, 0);
        assert_ne!(a, b);
    }
}
