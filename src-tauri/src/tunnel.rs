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
use std::os::unix::process::CommandExt as _;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

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
const PROCESS_EXIT_GRACE: Duration = Duration::from_millis(750);
const STDERR_SUMMARY_LIMIT: usize = 240;
static NEXT_SECRET_DIR: AtomicU64 = AtomicU64::new(0);

/// Loopback endpoints the planes must use instead of the LAN host.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct TunnelEndpoints {
    pub host: String,
    pub ssh_port: u16,
    pub rdp_port: u16,
}

/// How tunnels are routed in this process.
///
/// `Internal` (the release default) lets [`TunnelManager`] spawn one loopback
/// tunnel per remote DEVICE. `External` is an explicit single-device DEV debug
/// mode: a manually managed `ssh -L` tunnel is reused, and a second device
/// identity is rejected with `TUNNEL_EXTERNAL_SINGLE_DEVICE` instead of being
/// silently routed to the same Jetson.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TunnelMode {
    Internal,
    External { ssh_port: u16, rdp_port: u16 },
}

/// Legacy compile-time override detection (transition safety only). If a stale
/// binary still has `VITE_JR_SSH_PORT` baked in, we must say so loudly rather
/// than silently routing through a single external tunnel; the routing itself
/// no longer reads it (see [`tunnel_mode`]).
const LEGACY_COMPILED_EXTERNAL: Option<&str> = option_env!("VITE_JR_SSH_PORT");

#[derive(Debug, thiserror::Error)]
pub enum TunnelError {
    #[error("the external tunnel supports a single device only")]
    ExternalSingleDevice,
    #[error("could not reach the device")]
    Unreachable,
    #[error("authentication failed")]
    AuthFailed,
    #[error("local port allocation failed: {0}")]
    LocalPort(String),
    #[error("ssh tunnel exited: {0}")]
    SshExited(String),
    #[error("tunnel setup failed: {0}")]
    Setup(String),
}

impl TunnelError {
    /// Stable machine-readable classification for DEV diagnostics. Never
    /// contains credentials (the sanitized ssh stderr / port strings are the
    /// only dynamic parts). Callers may surface this verbatim.
    pub fn code(&self) -> &'static str {
        match self {
            TunnelError::ExternalSingleDevice => "TUNNEL_EXTERNAL_SINGLE_DEVICE",
            TunnelError::Unreachable => "TUNNEL_TARGET_UNREACHABLE",
            TunnelError::AuthFailed => "TUNNEL_AUTH_FAILED",
            TunnelError::LocalPort(_) => "TUNNEL_LOCAL_PORT_FAILED",
            TunnelError::SshExited(_) => "TUNNEL_SSH_EXITED",
            TunnelError::Setup(_) => "TUNNEL_SETUP_FAILED",
        }
    }
}

/// The remote board a tunnel terminates at. Two keys addressing the same
/// target are the same physical device (identity upgrade: a first-connect
/// tunnel keyed `user@host` is adopted into `user@deviceId` once known).
#[derive(Debug, Clone, PartialEq, Eq)]
struct TunnelTarget {
    host: String,
    port: u16,
    username: String,
}

struct ActiveTunnel {
    child: Child,
    process_group: libc::pid_t,
    endpoints: TunnelEndpoints,
    target: TunnelTarget,
    /// 0700 dir holding the askpass helper + password file; removed on drop.
    _secret_dir: SecretDir,
}

impl Drop for ActiveTunnel {
    fn drop(&mut self) {
        terminate_process_group(&mut self.child, self.process_group);
    }
}

struct SecretDir(PathBuf);

impl SecretDir {
    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for SecretDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[derive(Default)]
struct TunnelState {
    active: HashMap<String, ActiveTunnel>,
    external_target: Option<String>,
}

/// One tunnel per remote DEVICE (multi-device support, V0.4), keyed by the
/// stable per-device identity (`username@deviceId`). Each tunnel gets its own
/// loopback ports (preferred ports for the first one, ephemeral for the rest),
/// so several Jetsons can be connected simultaneously — while both addresses
/// of the SAME Jetson share one tunnel. Tunnels live for the app process
/// lifetime and are all killed on exit.
#[derive(Clone, Default)]
pub struct TunnelManager {
    inner: Arc<Mutex<TunnelState>>,
}

impl TunnelManager {
    pub fn new() -> Self {
        if let Some(root) = default_tunnel_root() {
            sweep_orphans(&root.join("known_hosts"));
            cleanup_stale_secret_dirs(&root);
        }
        // One-time diagnostic banner (never prints credentials): which mode is
        // this process in? A stale binary that still bakes in the legacy
        // compile-time override is reported loudly so it gets rebuilt.
        match tunnel_mode() {
            TunnelMode::Internal => eprintln!("[jr-flow] tunnel mode=internal"),
            TunnelMode::External { ssh_port, rdp_port } => {
                eprintln!("[jr-flow] tunnel mode=external ssh_port={ssh_port} rdp_port={rdp_port}");
            }
        }
        if let Some(legacy) = LEGACY_COMPILED_EXTERNAL {
            eprintln!(
                "[jr-flow] WARNING legacy compile-time VITE_JR_SSH_PORT={legacy} is baked into this binary; rebuild clean — the runtime override is JR_EXTERNAL_SSH_PORT"
            );
        }
        Self::default()
    }

    /// Terminate every tunnel (app exit). Idempotent.
    pub fn close_all(&self) {
        let mut guard = self.inner.lock().unwrap();
        guard.active.clear(); // Drop kills each child and removes its secret files.
        guard.external_target = None;
    }

    /// Ensure a tunnel to `host` exists and return the loopback endpoints.
    /// `device_key` is the stable per-device identity (`username@deviceId`,
    /// falling back to `username@host`) — tunnels are keyed by it so the two
    /// addresses of the same Jetson share one tunnel while two different
    /// Jetsons never do. Blocking (spawns ssh + polls ports); call from
    /// `spawn_blocking`.
    pub fn ensure(
        &self,
        app: &AppHandle,
        host: &str,
        remote_ssh_port: u16,
        device_key: &str,
        username: &str,
        password: &str,
    ) -> Result<TunnelEndpoints, TunnelError> {
        let key = device_key.to_string();

        // Dev override: a manually managed external tunnel can represent only
        // one remote target. Reject a second identity explicitly instead of
        // silently routing two device tabs to the same Jetson.
        if let Some((ssh, rdp)) = runtime_external_ports() {
            let mut guard = self.inner.lock().unwrap();
            eprintln!(
                "[jr-flow] tunnel ensure mode=external key={key} host={host} user={username} bound={}",
                guard.external_target.as_deref().unwrap_or("<none>")
            );
            match bind_external_target(&mut guard.external_target, &key) {
                Ok(()) => {}
                Err(TunnelError::ExternalSingleDevice) => {
                    eprintln!(
                        "[jr-flow] tunnel ensure result=rejected EXTERNAL_TUNNEL_SINGLE_DEVICE key={key}"
                    );
                    return Err(TunnelError::ExternalSingleDevice);
                }
                Err(e) => return Err(e),
            }
            return Ok(TunnelEndpoints {
                host: "127.0.0.1".into(),
                ssh_port: ssh,
                rdp_port: rdp,
            });
        }

        let target = TunnelTarget {
            host: host.to_string(),
            port: remote_ssh_port,
            username: username.to_string(),
        };
        let mut guard = self.inner.lock().unwrap();
        eprintln!(
            "[jr-flow] tunnel ensure key={key} target={host}:{remote_ssh_port} user={username} existing={}",
            guard.active.len()
        );
        // Pure decision (no health checks / spawn) — kept separate so the
        // key-vs-target adoption rules are unit-testable without ssh.
        let targets: HashMap<String, TunnelTarget> = guard
            .active
            .iter()
            .map(|(k, t)| (k.clone(), t.target.clone()))
            .collect();
        match classify_ensure(&targets, &key, &target) {
            EnsureClass::Reuse => {
                let t = guard.active.get_mut(&key).expect("classify said reuse");
                if is_healthy(t) {
                    eprintln!(
                        "[jr-flow] tunnel ensure result=reused ssh={} rdp={}",
                        t.endpoints.ssh_port, t.endpoints.rdp_port
                    );
                    return Ok(t.endpoints.clone());
                }
                // Stale tunnel for this device: drop kills the child + files;
                // tunnels to OTHER devices are untouched (multi-device).
                eprintln!("[jr-flow] tunnel ensure result=stale key={key}");
                guard.active.remove(&key);
            }
            EnsureClass::Adopt { old_key } => {
                // Identity adoption: a healthy tunnel to the SAME board may
                // live under a different key (first connect keyed `user@host`
                // before the serial was known; this call carries
                // `user@deviceId`). Re-key it instead of spawning a second ssh.
                if let Some(mut t) = guard.active.remove(&old_key) {
                    if is_healthy(&mut t) {
                        eprintln!(
                            "[jr-flow] tunnel ensure result=adopted {old_key}->{key} ssh={} rdp={}",
                            t.endpoints.ssh_port, t.endpoints.rdp_port
                        );
                        let endpoints = t.endpoints.clone();
                        guard.active.insert(key, t);
                        return Ok(endpoints);
                    }
                    // Unhealthy: the drop above killed the child; spawn fresh.
                }
            }
            EnsureClass::Fresh => {}
        }

        eprintln!("[jr-flow] tunnel ensure result=spawning key={key} host={host} user={username}");
        let tunnel = spawn_tunnel(app, host, remote_ssh_port, username, password).map_err(|e| {
            // Every spawn failure lands here (pre-loop setup errors AND
            // loop exits) — the log is the only post-mortem surface.
            eprintln!(
                "[jr-flow] tunnel ensure result=failed code={}: {e}",
                e.code()
            );
            e
        })?;
        let endpoints = tunnel.endpoints.clone();
        guard.active.insert(key, tunnel);
        Ok(endpoints)
    }
}

fn bind_external_target(slot: &mut Option<String>, key: &str) -> Result<(), TunnelError> {
    match slot {
        Some(existing) if existing != key => Err(TunnelError::ExternalSingleDevice),
        Some(_) => Ok(()),
        None => {
            *slot = Some(key.to_owned());
            Ok(())
        }
    }
}

/// Pure decision made before any health/spawn work. Unit-testable without a
/// live ssh child or AppHandle: how `ensure` should treat `key`/`target` given
/// the current tunnel table (key → target)?
#[derive(Debug, Clone, PartialEq, Eq)]
enum EnsureClass {
    /// The key already owns a tunnel; the caller re-checks its health.
    Reuse,
    /// The same physical target lives under another key; the caller re-checks
    /// health before re-keying (identity upgrade: `user@host` → `user@deviceId`).
    Adopt { old_key: String },
    /// No existing tunnel for this key or its exact target; spawn fresh.
    Fresh,
}

fn classify_ensure(
    active: &HashMap<String, TunnelTarget>,
    key: &str,
    target: &TunnelTarget,
) -> EnsureClass {
    if active.contains_key(key) {
        return EnsureClass::Reuse;
    }
    if let Some((old_key, _)) = active.iter().find(|(_, t)| *t == target) {
        return EnsureClass::Adopt {
            old_key: old_key.clone(),
        };
    }
    EnsureClass::Fresh
}

/// Kill orphaned tunnel `ssh -N` processes left behind by a previous app
/// instance that was force-killed or crashed (normal exit reaps them via
/// RunEvent::Exit + Drop). Orphans hold the preferred local ports, which
/// would force ephemeral ports and churn the host-key trust store. The
/// match is safe: only our tunnel ssh carries the app-scoped known_hosts
/// path in its command line.
fn default_tunnel_root() -> Option<PathBuf> {
    let home = std::env::var_os("HOME")?;
    Some(PathBuf::from(home).join("Library/Application Support/com.jetsonremote.app/tunnel"))
}

fn sweep_orphans(known_hosts: &Path) {
    let known = known_hosts.to_string_lossy();
    let Ok(out) = Command::new("/usr/bin/pgrep")
        .arg("-f")
        .arg(known.as_ref())
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
            signal_process_group(pid, libc::SIGTERM);
        }
    }
}

fn cleanup_stale_secret_dirs(root: &Path) {
    let Ok(entries) = fs::read_dir(root) else {
        return;
    };
    for entry in entries.flatten() {
        let name = entry.file_name();
        if name.to_string_lossy().starts_with("session-") {
            let _ = fs::remove_dir_all(entry.path());
        }
    }
}

/// Current tunnel routing mode for this process. Runtime-only (release never
/// sets the env, so its default is always Internal multi-device). DEV overrides
/// use `JR_EXTERNAL_SSH_PORT` (optional `JR_EXTERNAL_RDP_PORT`, default 3389).
pub fn tunnel_mode() -> TunnelMode {
    match runtime_external_ports() {
        Some((ssh_port, rdp_port)) => TunnelMode::External { ssh_port, rdp_port },
        None => TunnelMode::Internal,
    }
}

/// Runtime-only DEV override: `JR_EXTERNAL_SSH_PORT=2222` rides an externally
/// managed tunnel (the pre-0.2.1 single-tunnel debug workflow). Replaces the
/// old compile-time `option_env!("VITE_JR_SSH_PORT")` so that merely unsetting
/// a shell variable can never leave a stale binary in the wrong mode.
fn runtime_external_ports() -> Option<(u16, u16)> {
    let ssh: u16 = std::env::var("JR_EXTERNAL_SSH_PORT")
        .ok()
        .and_then(|v| v.trim().parse().ok())?;
    let rdp = std::env::var("JR_EXTERNAL_RDP_PORT")
        .ok()
        .and_then(|v| v.trim().parse().ok())
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
        .map_err(|e| TunnelError::LocalPort(format!("no free local port: {e}")))
        .and_then(|p| {
            if p == 0 {
                Err(TunnelError::LocalPort("no free local port".into()))
            } else {
                Ok(p)
            }
        })
}

fn pick_ports() -> Result<(u16, u16), TunnelError> {
    if port_free(PREFERRED_SSH_PORT) && port_free(PREFERRED_RDP_PORT) {
        return Ok((PREFERRED_SSH_PORT, PREFERRED_RDP_PORT));
    }
    let ssh = free_port()?;
    for _ in 0..8 {
        let rdp = free_port()?;
        if rdp != ssh {
            return Ok((ssh, rdp));
        }
    }
    Err(TunnelError::LocalPort(
        "could not allocate distinct local ports".into(),
    ))
}

#[cfg(unix)]
fn chmod(path: &std::path::Path, mode: u32) -> Result<(), TunnelError> {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(path, fs::Permissions::from_mode(mode))
        .map_err(|e| TunnelError::Setup(format!("chmod {}: {e}", path.display())))
}

fn create_secret_dir(root: &Path) -> Result<SecretDir, TunnelError> {
    let epoch = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    for _ in 0..16 {
        let sequence = NEXT_SECRET_DIR.fetch_add(1, Ordering::Relaxed);
        let path = root.join(format!("session-{}-{epoch}-{sequence}", std::process::id()));
        match fs::create_dir(&path) {
            Ok(()) => {
                let secret_dir = SecretDir(path);
                chmod(secret_dir.path(), 0o700)?;
                return Ok(secret_dir);
            }
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => {
                return Err(TunnelError::Setup(format!(
                    "create tunnel credential directory: {e}"
                )))
            }
        }
    }
    Err(TunnelError::Setup(
        "could not allocate tunnel credential directory".into(),
    ))
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
    let root = base.join("tunnel");
    fs::create_dir_all(&root).map_err(|e| TunnelError::Setup(format!("mkdir: {e}")))?;
    chmod(&root, 0o700)?;
    let secret_dir = create_secret_dir(&root)?;

    // Password reaches ssh only via the askpass helper (never argv).
    let pw_file = secret_dir.path().join("pw");
    fs::write(&pw_file, format!("{password}\n"))
        .map_err(|e| TunnelError::Setup(format!("write pw: {e}")))?;
    chmod(&pw_file, 0o600)?;
    let askpass = secret_dir.path().join("askpass.sh");
    fs::write(
        &askpass,
        format!("#!/bin/sh\ncat \"{}\"\n", pw_file.display()),
    )
    .map_err(|e| TunnelError::Setup(format!("write askpass: {e}")))?;
    chmod(&askpass, 0o700)?;
    let known_hosts = root.join("known_hosts");

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
    cmd.process_group(0);

    let mut child = cmd
        .spawn()
        .map_err(|e| TunnelError::Setup(format!("spawn {SSH_BIN}: {e}")))?;
    let process_group = child.id() as libc::pid_t;

    // Local -L listeners open before auth completes, so "ports open" alone is
    // not success: after they open, wait AUTH_GRACE for ssh to die on an auth
    // failure; surviving the grace means authentication went through.
    let deadline = Instant::now() + READY_TIMEOUT;
    loop {
        if let Some(status) = child
            .try_wait()
            .map_err(|e| TunnelError::Setup(format!("wait: {e}")))?
        {
            terminate_process_group(&mut child, process_group);
            let stderr = drain_stderr(&mut child);
            eprintln!(
                "[jr-flow] tunnel ssh exited {:?}: {}",
                status.code(),
                stderr_summary(&stderr)
            );
            return Err(classify_exit(&stderr, status.code()));
        }
        if port_open(ssh_port) && port_open(rdp_port) {
            let grace_end = Instant::now() + AUTH_GRACE;
            let mut failed = None;
            while Instant::now() < grace_end {
                std::thread::sleep(Duration::from_millis(200));
                if let Ok(Some(status)) = child.try_wait() {
                    terminate_process_group(&mut child, process_group);
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
                        process_group,
                        endpoints: TunnelEndpoints {
                            host: "127.0.0.1".into(),
                            ssh_port,
                            rdp_port,
                        },
                        target: TunnelTarget {
                            host: host.to_string(),
                            port: remote_ssh_port,
                            username: username.to_string(),
                        },
                        _secret_dir: secret_dir,
                    });
                }
            }
        }
        if Instant::now() >= deadline {
            // stderr remains open while ssh is alive. Reap it before draining
            // or a timeout itself can block forever on read_to_string.
            let stderr = terminate_and_drain_stderr(&mut child, process_group);
            eprintln!(
                "[jr-flow] tunnel deadline; ssh stderr: {}",
                stderr_summary(&stderr)
            );
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

fn terminate_and_drain_stderr(child: &mut Child, process_group: libc::pid_t) -> String {
    terminate_process_group(child, process_group);
    drain_stderr(child)
}

fn signal_process_group(process_group: libc::pid_t, signal: libc::c_int) {
    if process_group > 1 {
        unsafe {
            if libc::kill(-process_group, signal) != 0 {
                // Releases before 0.3.2 did not create a process group. Keep
                // startup orphan cleanup compatible with those SSH children.
                libc::kill(process_group, signal);
            }
        }
    }
}

fn process_group_alive(process_group: libc::pid_t) -> bool {
    process_group > 1 && unsafe { libc::kill(-process_group, 0) == 0 }
}

fn terminate_process_group(child: &mut Child, process_group: libc::pid_t) {
    signal_process_group(process_group, libc::SIGTERM);
    let deadline = Instant::now() + PROCESS_EXIT_GRACE;
    while Instant::now() < deadline {
        let _ = child.try_wait();
        if !process_group_alive(process_group) {
            break;
        }
        std::thread::sleep(Duration::from_millis(25));
    }
    if process_group_alive(process_group) {
        signal_process_group(process_group, libc::SIGKILL);
    }
    let _ = child.wait();
}

/// Keep process diagnostics useful without dumping unbounded stderr into the
/// application log. SSH never receives a password on stderr, but the summary
/// still avoids logging more than one user-facing failure line.
fn stderr_summary(stderr: &str) -> String {
    stderr
        .lines()
        .find(|line| !line.trim().is_empty())
        .map(str::trim)
        .unwrap_or("no diagnostic output")
        .chars()
        .take(STDERR_SUMMARY_LIMIT)
        .collect()
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
    TunnelError::SshExited(format!(
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
            TunnelError::SshExited(m) => assert!(m.contains("Address already in use")),
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn tunnel_error_codes_are_stable_and_distinct() {
        assert_eq!(
            TunnelError::ExternalSingleDevice.code(),
            "TUNNEL_EXTERNAL_SINGLE_DEVICE"
        );
        assert_eq!(TunnelError::Unreachable.code(), "TUNNEL_TARGET_UNREACHABLE");
        assert_eq!(TunnelError::AuthFailed.code(), "TUNNEL_AUTH_FAILED");
        assert_eq!(
            TunnelError::LocalPort("x".into()).code(),
            "TUNNEL_LOCAL_PORT_FAILED"
        );
        assert_eq!(
            TunnelError::SshExited("x".into()).code(),
            "TUNNEL_SSH_EXITED"
        );
        // Distinct codes for distinct causes — never two variants sharing one.
        let codes = [
            TunnelError::ExternalSingleDevice.code(),
            TunnelError::Unreachable.code(),
            TunnelError::AuthFailed.code(),
            TunnelError::LocalPort("x".into()).code(),
            TunnelError::SshExited("x".into()).code(),
        ];
        for (i, a) in codes.iter().enumerate() {
            for b in &codes[i + 1..] {
                assert_ne!(a, b);
            }
        }
    }

    #[test]
    fn classify_ensure_multiple_devices_never_replace_each_other() {
        // Device A and B have distinct deviceIds → distinct keys → each gets
        // its own Fresh slot; adding B must never Reuse/Adopt A.
        let mut active = HashMap::new();
        active.insert(
            "seeed@serialA".to_string(),
            TunnelTarget {
                host: "192.168.1.10".into(),
                port: 22,
                username: "seeed".into(),
            },
        );
        let b = TunnelTarget {
            host: "192.168.1.11".into(),
            port: 22,
            username: "seeed".into(),
        };
        assert_eq!(
            classify_ensure(&active, "seeed@serialB", &b),
            EnsureClass::Fresh
        );
        // A remains untouched (the caller only mutates under `Fresh`).
        assert_eq!(active.len(), 1);
        assert!(active.contains_key("seeed@serialA"));
    }

    #[test]
    fn classify_ensure_reuses_same_key_and_adopts_same_target() {
        let mut active = HashMap::new();
        let target = TunnelTarget {
            host: "192.168.1.10".into(),
            port: 22,
            username: "seeed".into(),
        };
        active.insert("seeed@192.168.1.10".to_string(), target.clone());

        // Same key → reuse (health re-checked by caller).
        assert_eq!(
            classify_ensure(&active, "seeed@192.168.1.10", &target),
            EnsureClass::Reuse
        );
        // Same physical target under a NEW key (`user@host` → `user@serial`)
        // → adoption, never a second tunnel to one board.
        assert_eq!(
            classify_ensure(&active, "seeed@serialA", &target),
            EnsureClass::Adopt {
                old_key: "seeed@192.168.1.10".into()
            }
        );
    }

    #[test]
    fn classify_ensure_distinct_targets_never_adopt() {
        let mut active = HashMap::new();
        active.insert(
            "seeed@serialA".to_string(),
            TunnelTarget {
                host: "192.168.1.10".into(),
                port: 22,
                username: "seeed".into(),
            },
        );
        // Different host, different port, different username → all Fresh.
        for target in [
            TunnelTarget {
                host: "192.168.1.11".into(),
                port: 22,
                username: "seeed".into(),
            },
            TunnelTarget {
                host: "192.168.1.10".into(),
                port: 2222,
                username: "seeed".into(),
            },
            TunnelTarget {
                host: "192.168.1.10".into(),
                port: 22,
                username: "root".into(),
            },
        ] {
            assert_eq!(
                classify_ensure(&active, "seeed@serialB", &target),
                EnsureClass::Fresh
            );
        }
    }

    #[test]
    fn pick_ports_returns_free_ports() {
        let (a, b) = pick_ports().unwrap();
        assert_ne!(a, 0);
        assert_ne!(b, 0);
        assert_ne!(a, b);
    }

    #[test]
    fn timeout_cleanup_reaps_before_reading_stderr() {
        let mut child = Command::new("/bin/sh")
            .args(["-c", "printf 'still running\\n' >&2; sleep 30"])
            .stderr(Stdio::piped())
            .process_group(0)
            .spawn()
            .unwrap();
        let process_group = child.id() as libc::pid_t;
        std::thread::sleep(Duration::from_millis(50));
        let started = Instant::now();
        let stderr = terminate_and_drain_stderr(&mut child, process_group);

        assert!(started.elapsed() < Duration::from_secs(2));
        assert_eq!(stderr_summary(&stderr), "still running");
    }

    #[test]
    fn tunnel_target_identifies_one_board_across_keys() {
        // Identity adoption: the same board+user is the same target no matter
        // which key (user@host vs user@deviceId) the tunnel was created under.
        let first = TunnelTarget {
            host: "192.168.2.18".into(),
            port: 22,
            username: "seeed".into(),
        };
        assert_eq!(
            first,
            TunnelTarget {
                host: "192.168.2.18".into(),
                port: 22,
                username: "seeed".into()
            }
        );
        // A different board (or entry address) is a different target — never
        // adopted across devices.
        assert_ne!(
            first,
            TunnelTarget {
                host: "100.114.170.49".into(),
                port: 22,
                username: "seeed".into()
            }
        );
        assert_ne!(
            first,
            TunnelTarget {
                host: "192.168.2.18".into(),
                port: 22,
                username: "other".into()
            }
        );
    }

    #[test]
    fn external_tunnel_is_bound_to_one_remote_identity() {
        let mut target = None;
        bind_external_target(&mut target, "seeed@jetson-a:22").unwrap();
        bind_external_target(&mut target, "seeed@jetson-a:22").unwrap();
        assert!(bind_external_target(&mut target, "seeed@jetson-b:22").is_err());
    }

    #[test]
    fn secret_directories_are_isolated_and_drop_independently() {
        let root = std::env::temp_dir().join(format!(
            "jetson-remote-tunnel-test-{}-{}",
            std::process::id(),
            NEXT_SECRET_DIR.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&root).unwrap();
        let first = create_secret_dir(&root).unwrap();
        let first_path = first.path().to_owned();
        let second = create_secret_dir(&root).unwrap();
        let second_path = second.path().to_owned();

        assert_ne!(first_path, second_path);
        drop(first);
        assert!(!first_path.exists());
        assert!(second_path.exists());
        drop(second);
        assert!(!second_path.exists());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn stderr_summary_is_bounded() {
        let summary = stderr_summary(&"x".repeat(STDERR_SUMMARY_LIMIT + 20));
        assert_eq!(summary.chars().count(), STDERR_SUMMARY_LIMIT);
    }
}
