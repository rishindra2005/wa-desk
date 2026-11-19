//! Runs the bundled WhatsApp bridge (the `wa-bridge` sidecar) when needed.
//!
//! - [`decide`] is the pure startup decision (mode, is a bridge already
//!   answering, is the binary installed).
//! - [`RestartPolicy`] is the pure backoff policy (1 s, 2 s, 5 s, ... 60 s;
//!   gives up after repeated fast crashes).
//! - [`Supervisor`] drives them against a [`Spawner`] and a [`Probe`], so the
//!   whole loop is testable with fakes. [`TokioSpawner`] is the real one.
//!
//! The bridge keeps its data in `store/` under its working directory, so it
//! runs with cwd = `<bridge_store>/..` and uses exactly wa-desk's store.

use std::io;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use parking_lot::Mutex;
use tokio::io::{AsyncBufReadExt, AsyncRead, BufReader};
use tokio_util::sync::CancellationToken;
use wa_core::types::{BridgeMode, SupervisorState, SupervisorStatus};

/// Name of the sidecar next to the app executable.
pub const SIDECAR_NAME: &str = if cfg!(windows) {
    "wa-bridge.exe"
} else {
    "wa-bridge"
};
/// Dev override for the sidecar path.
pub const BIN_ENV: &str = "WA_DESK_BRIDGE_BIN";
/// File name of the bridge's output log inside the app logs folder.
pub const LOG_NAME: &str = "bridge.log";
/// Size at which `bridge.log` rotates to `bridge.log.1`.
pub const LOG_CAP: u64 = 5 * 1024 * 1024;
/// How long a bridge gets to exit after the polite stop request.
pub const STOP_GRACE: Duration = Duration::from_secs(5);

/// Restart delays by consecutive short run; the last one repeats.
const DELAYS: [Duration; 7] = [
    Duration::from_secs(1),
    Duration::from_secs(2),
    Duration::from_secs(5),
    Duration::from_secs(10),
    Duration::from_secs(20),
    Duration::from_secs(40),
    Duration::from_secs(60),
];
/// A run at least this long counts as healthy and resets the backoff.
const STABLE_RUN: Duration = Duration::from_secs(30);
/// Consecutive short runs tolerated before giving up.
const MAX_FAST_CRASHES: u32 = 8;

// ---------------------------------------------------------------- decision

/// What to do at startup.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Decision {
    /// A bridge already answers (auto mode): leave it alone.
    External,
    /// Never spawn: `bridge_mode = external`, or the bridge URL is not local.
    Disabled,
    /// No bundled binary in this install.
    NotInstalled,
    /// `bridge_mode = bundled` but something already holds the port.
    PortBusy,
    /// Start the bundled bridge.
    Spawn,
}

/// Pure startup decision.
pub fn decide(
    mode: BridgeMode,
    url_is_local: bool,
    reachable: bool,
    binary_exists: bool,
) -> Decision {
    if mode == BridgeMode::External || !url_is_local {
        return Decision::Disabled;
    }
    match mode {
        BridgeMode::Auto if reachable => Decision::External,
        _ if !binary_exists => Decision::NotInstalled,
        BridgeMode::Bundled if reachable => Decision::PortBusy,
        _ => Decision::Spawn,
    }
}

/// Host and port of the bridge URL, and whether the host is this machine.
pub fn parse_bridge_url(url: &str) -> Option<(String, u16, bool)> {
    let u = url::Url::parse(url).ok()?;
    let host = u.host_str()?.to_string();
    let port = u.port_or_known_default()?;
    let local = host == "localhost"
        || host
            .trim_matches(['[', ']'])
            .parse::<std::net::IpAddr>()
            .is_ok_and(|ip| ip.is_loopback());
    Some((host, port, local))
}

/// Where the sidecar should be: `WA_DESK_BRIDGE_BIN`, else next to the executable.
pub fn sidecar_path() -> Option<PathBuf> {
    if let Some(p) = std::env::var_os(BIN_ENV).filter(|v| !v.is_empty()) {
        return Some(PathBuf::from(p));
    }
    Some(std::env::current_exe().ok()?.parent()?.join(SIDECAR_NAME))
}

// ------------------------------------------------------------------ policy

/// Backoff and give-up policy for a bridge that keeps exiting.
#[derive(Debug, Default)]
pub struct RestartPolicy {
    streak: u32,
}

impl RestartPolicy {
    /// The bridge exited after running for `ran_for`. `Some(delay)` = restart
    /// after that delay; `None` = give up.
    pub fn on_exit(&mut self, ran_for: Duration) -> Option<Duration> {
        if ran_for >= STABLE_RUN {
            self.streak = 0;
        }
        self.streak += 1;
        if self.streak > MAX_FAST_CRASHES {
            return None;
        }
        Some(DELAYS[(self.streak as usize - 1).min(DELAYS.len() - 1)])
    }
}

// --------------------------------------------------------------------- log

/// Append-only log with one rotated generation (`<name>.1`).
pub struct RotatingLog {
    path: PathBuf,
    cap: u64,
    inner: Mutex<Option<(std::fs::File, u64)>>,
}

impl RotatingLog {
    /// Log at `path`, rotating when it would exceed `cap` bytes.
    pub fn new(path: PathBuf, cap: u64) -> Self {
        Self {
            path,
            cap,
            inner: Mutex::new(None),
        }
    }

    fn open(path: &Path) -> io::Result<(std::fs::File, u64)> {
        let mut o = std::fs::OpenOptions::new();
        o.create(true).append(true);
        #[cfg(unix)]
        std::os::unix::fs::OpenOptionsExt::mode(&mut o, 0o600);
        let f = o.open(path)?;
        let len = f.metadata()?.len();
        Ok((f, len))
    }

    /// Append one line (newline added).
    pub fn write_line(&self, line: &str) -> io::Result<()> {
        use std::io::Write;
        let mut g = self.inner.lock();
        if g.is_none() {
            *g = Some(Self::open(&self.path)?);
        }
        let need = line.len() as u64 + 1;
        let over = g
            .as_ref()
            .is_some_and(|(_, n)| *n > 0 && *n + need > self.cap);
        if over {
            *g = None;
            let mut rotated = self.path.clone().into_os_string();
            rotated.push(".1");
            std::fs::rename(&self.path, rotated)?;
            *g = Some(Self::open(&self.path)?);
        }
        if let Some((f, n)) = g.as_mut() {
            writeln!(f, "{line}")?;
            *n += need;
        }
        Ok(())
    }
}

/// Lines that must never reach the log: the auth token banner and QR art.
fn is_sensitive(line: &str) -> bool {
    line.contains("Token:")
        || line.contains("WHATSAPP_BRIDGE_TOKEN")
        || line.contains('\u{2588}')
        || line.contains('\u{2584}')
        || line.contains('\u{2580}')
}

async fn pump<R: AsyncRead + Unpin>(r: R, log: Arc<RotatingLog>) {
    let mut r = BufReader::new(r);
    let mut buf = Vec::new();
    loop {
        buf.clear();
        match r.read_until(b'\n', &mut buf).await {
            Ok(0) | Err(_) => return,
            Ok(_) => {
                let s = String::from_utf8_lossy(&buf);
                let s = s.trim_end();
                if !s.is_empty() && !is_sensitive(s) {
                    let _ = log.write_line(s);
                }
            }
        }
    }
}

// ------------------------------------------------------------------ traits

/// How to start the bridge.
#[derive(Debug, Clone)]
pub struct SpawnSpec {
    /// Executable to run.
    pub program: PathBuf,
    /// Command-line arguments.
    pub args: Vec<String>,
    /// Working directory (`<bridge_store>/..` for the bridge).
    pub cwd: PathBuf,
    /// Extra environment.
    pub env: Vec<(String, String)>,
}

/// A started bridge process.
#[async_trait::async_trait]
pub trait RunningChild: Send {
    /// Process id, if known.
    fn pid(&self) -> Option<u32>;
    /// Wait for it to exit; returns a short description of how.
    async fn wait(&mut self) -> String;
    /// Ask it to stop, then force it after `grace`.
    async fn terminate(&mut self, grace: Duration);
}

/// Starts bridge processes.
pub trait Spawner: Send + Sync {
    /// Start the process; its output goes to `log`.
    fn spawn(&self, spec: &SpawnSpec, log: Arc<RotatingLog>) -> io::Result<Box<dyn RunningChild>>;
}

/// Tells whether something already answers on the bridge address.
#[async_trait::async_trait]
pub trait Probe: Send + Sync {
    /// True if a bridge (or anything) accepts connections.
    async fn is_up(&self) -> bool;
}

/// Real probe: a TCP connect to the bridge host and port.
pub struct TcpProbe {
    /// Host from the bridge URL.
    pub host: String,
    /// Port from the bridge URL.
    pub port: u16,
}

#[async_trait::async_trait]
impl Probe for TcpProbe {
    async fn is_up(&self) -> bool {
        let c = tokio::net::TcpStream::connect((self.host.as_str(), self.port));
        matches!(
            tokio::time::timeout(Duration::from_millis(800), c).await,
            Ok(Ok(_))
        )
    }
}

/// systemd user unit that, when enabled or active, means the user runs the
/// bridge themselves (the laptop setup). Auto mode then never spawns.
pub const SYSTEMD_UNIT: &str = "whatsapp-bridge.service";
/// Name of the advisory lock file next to the bridge's `store/`.
pub const LOCK_NAME: &str = ".wa-desk-bridge.lock";
/// Auto mode probes the port this many times before deciding to spawn.
pub const RECHECKS: u32 = 3;
/// Gap between those probes (3 probes span about 6 s).
pub const RECHECK_GAP: Duration = Duration::from_secs(3);

/// Asks whether the bridge is managed by something else (systemd).
#[async_trait::async_trait]
pub trait Systemd: Send + Sync {
    /// True if the bridge's service unit is enabled or active.
    async fn bridge_unit_present(&self) -> bool;
}

/// Real check: `systemctl --user is-enabled|is-active whatsapp-bridge.service`.
/// A missing `systemctl` (or non-Linux) counts as "not present".
pub struct SystemctlCheck {
    /// The `systemctl` program (a fake in tests).
    pub program: PathBuf,
}

impl Default for SystemctlCheck {
    fn default() -> Self {
        Self {
            program: PathBuf::from("systemctl"),
        }
    }
}

#[async_trait::async_trait]
impl Systemd for SystemctlCheck {
    async fn bridge_unit_present(&self) -> bool {
        if !cfg!(target_os = "linux") {
            return false;
        }
        for verb in ["is-enabled", "is-active"] {
            let mut cmd = tokio::process::Command::new(&self.program);
            cmd.args(["--user", verb, SYSTEMD_UNIT])
                .stdin(std::process::Stdio::null())
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .kill_on_drop(true);
            let ok = match tokio::time::timeout(Duration::from_secs(2), cmd.status()).await {
                Ok(Ok(s)) => s.success(),
                _ => false,
            };
            if ok {
                return true;
            }
        }
        false
    }
}

/// Sleeps; replaced by a recording fake in tests.
#[async_trait::async_trait]
pub trait Clock: Send + Sync {
    /// Wait for `d`.
    async fn sleep(&self, d: Duration);
}

/// Real clock.
pub struct TokioClock;

#[async_trait::async_trait]
impl Clock for TokioClock {
    async fn sleep(&self, d: Duration) {
        tokio::time::sleep(d).await;
    }
}

/// Exclusive advisory lock on the bridge store; released on drop.
#[derive(Debug)]
pub struct StoreLock {
    // Held only so the OS lock lives as long as this value.
    _file: std::fs::File,
}

impl StoreLock {
    /// Try to take the lock at `path` (parent created owner-only).
    /// `Ok(None)` means another holder has it.
    pub fn try_acquire(path: &Path) -> io::Result<Option<Self>> {
        if let Some(dir) = path.parent() {
            ensure_private_dir(dir)?;
        }
        let mut o = std::fs::OpenOptions::new();
        o.create(true).truncate(false).write(true);
        #[cfg(unix)]
        std::os::unix::fs::OpenOptionsExt::mode(&mut o, 0o600);
        let f = o.open(path)?;
        match f.try_lock() {
            Ok(()) => Ok(Some(Self { _file: f })),
            Err(std::fs::TryLockError::WouldBlock) => Ok(None),
            Err(std::fs::TryLockError::Error(e)) => Err(e),
        }
    }
}

/// Real spawner built on `tokio::process`.
pub struct TokioSpawner;

struct TokioChild(tokio::process::Child);

#[async_trait::async_trait]
impl RunningChild for TokioChild {
    fn pid(&self) -> Option<u32> {
        self.0.id()
    }

    async fn wait(&mut self) -> String {
        match self.0.wait().await {
            Ok(s) => format!("exited with {s}"),
            Err(e) => format!("wait failed: {e}"),
        }
    }

    async fn terminate(&mut self, grace: Duration) {
        #[cfg(unix)]
        if let Some(pid) = self.0.id().and_then(|p| i32::try_from(p).ok()) {
            let _ = nix::sys::signal::kill(
                nix::unistd::Pid::from_raw(pid),
                nix::sys::signal::Signal::SIGTERM,
            );
            if tokio::time::timeout(grace, self.0.wait()).await.is_ok() {
                return;
            }
        }
        #[cfg(not(unix))]
        let _ = grace;
        let _ = self.0.kill().await;
    }
}

impl Spawner for TokioSpawner {
    fn spawn(&self, spec: &SpawnSpec, log: Arc<RotatingLog>) -> io::Result<Box<dyn RunningChild>> {
        let mut cmd = tokio::process::Command::new(&spec.program);
        cmd.args(&spec.args)
            .current_dir(&spec.cwd)
            .envs(spec.env.iter().map(|(k, v)| (k, v)))
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .kill_on_drop(true);
        #[cfg(windows)]
        cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
        let mut child = cmd.spawn()?;
        if let Some(o) = child.stdout.take() {
            tokio::spawn(pump(o, log.clone()));
        }
        if let Some(e) = child.stderr.take() {
            tokio::spawn(pump(e, log));
        }
        Ok(Box::new(TokioChild(child)))
    }
}

/// The store lock file: next to `store/`, in the bridge's working directory.
pub fn lock_path(bridge_store: &Path) -> PathBuf {
    bridge_store
        .parent()
        .unwrap_or(bridge_store)
        .join(LOCK_NAME)
}

/// Create `dir` (and parents) owner-only if missing.
pub fn ensure_private_dir(dir: &Path) -> io::Result<()> {
    let mut b = std::fs::DirBuilder::new();
    b.recursive(true);
    #[cfg(unix)]
    std::os::unix::fs::DirBuilderExt::mode(&mut b, 0o700);
    b.create(dir)
}

// -------------------------------------------------------------- supervisor

/// Everything the supervisor needs to know.
#[derive(Debug, Clone)]
pub struct Config {
    /// `Settings.bridge_mode`.
    pub mode: BridgeMode,
    /// Bridge host and port; `None` if the URL could not be parsed.
    pub addr: Option<(String, u16, bool)>,
    /// Sidecar binary path, if this install can have one.
    pub binary: Option<PathBuf>,
    /// The bridge store (`<data>/whatsapp-mcp/runtime/store`).
    pub bridge_store: PathBuf,
    /// App logs folder (holds `bridge.log`).
    pub logs: PathBuf,
}

/// Called with (title, detail) when the supervisor gives up.
pub type IssueFn = Arc<dyn Fn(&str, &str) + Send + Sync>;

/// Owns the bundled bridge process and publishes its state.
pub struct Supervisor {
    status: Mutex<SupervisorStatus>,
    cancel: CancellationToken,
    finished: CancellationToken,
    started: AtomicBool,
    /// Delay scale for tests (1.0 in the app).
    delays: Mutex<Option<Duration>>,
    systemd: Mutex<Arc<dyn Systemd>>,
    clock: Mutex<Arc<dyn Clock>>,
}

impl Default for Supervisor {
    fn default() -> Self {
        Self::new()
    }
}

impl Supervisor {
    /// A supervisor that has not decided anything yet.
    pub fn new() -> Self {
        Self {
            status: Mutex::new(SupervisorStatus {
                state: SupervisorState::Starting,
                pid: None,
                detail: None,
                restarts: 0,
            }),
            cancel: CancellationToken::new(),
            finished: CancellationToken::new(),
            started: AtomicBool::new(false),
            delays: Mutex::new(None),
            systemd: Mutex::new(Arc::new(SystemctlCheck::default())),
            clock: Mutex::new(Arc::new(TokioClock)),
        }
    }

    /// Replace the systemd check (tests).
    pub fn set_systemd(&self, s: Arc<dyn Systemd>) {
        *self.systemd.lock() = s;
    }

    /// Replace the clock used for the auto-mode re-checks (tests).
    pub fn set_clock(&self, c: Arc<dyn Clock>) {
        *self.clock.lock() = c;
    }

    /// Replace every restart delay with `d` (tests only).
    pub fn fixed_delay(&self, d: Duration) {
        *self.delays.lock() = Some(d);
    }

    /// Current state.
    pub fn status(&self) -> SupervisorStatus {
        self.status.lock().clone()
    }

    fn set(&self, state: SupervisorState, pid: Option<u32>, detail: Option<String>) {
        let mut s = self.status.lock();
        s.state = state;
        s.pid = pid;
        s.detail = detail;
    }

    /// Stop the bridge (if running) and the loop; returns when it is gone.
    pub async fn stop(&self) {
        self.cancel.cancel();
        if self.started.load(Ordering::SeqCst) {
            let _ = tokio::time::timeout(STOP_GRACE * 2, self.finished.cancelled()).await;
        }
    }

    /// Decide and, if needed, run the bridge until [`Self::stop`].
    pub async fn run(
        &self,
        cfg: Config,
        spawner: Arc<dyn Spawner>,
        probe: Arc<dyn Probe>,
        on_issue: IssueFn,
    ) {
        self.started.store(true, Ordering::SeqCst);
        self.run_inner(cfg, spawner, probe, on_issue).await;
        self.finished.cancel();
    }

    async fn run_inner(
        &self,
        cfg: Config,
        spawner: Arc<dyn Spawner>,
        probe: Arc<dyn Probe>,
        on_issue: IssueFn,
    ) {
        if self.cancel.is_cancelled() {
            self.set(SupervisorState::Stopped, None, None);
            return;
        }
        let (port, local) = cfg.addr.as_ref().map_or((0, false), |a| (a.1, a.2));
        let have_bin = cfg.binary.as_ref().is_some_and(|b| b.is_file());
        if cfg.mode == BridgeMode::Auto && local && have_bin {
            let sd = self.systemd.lock().clone();
            if sd.bridge_unit_present().await {
                tracing::info!(
                    unit = SYSTEMD_UNIT,
                    "bridge is managed by systemd; not spawning"
                );
                self.set(
                    SupervisorState::External,
                    None,
                    Some(format!("Managed by the systemd user unit {SYSTEMD_UNIT}.")),
                );
                return;
            }
        }
        let mut reachable = local && probe.is_up().await;
        if cfg.mode == BridgeMode::Auto && local && have_bin {
            // A bridge that is still starting may not answer yet.
            let clock = self.clock.lock().clone();
            for _ in 1..RECHECKS {
                if reachable {
                    break;
                }
                clock.sleep(RECHECK_GAP).await;
                reachable = probe.is_up().await;
            }
        }
        let d = decide(cfg.mode, local, reachable, have_bin);
        tracing::info!(?d, mode = ?cfg.mode, reachable, have_bin, "bridge supervisor decision");
        let (state, detail) = match d {
            Decision::Spawn => {
                let lock = match StoreLock::try_acquire(&lock_path(&cfg.bridge_store)) {
                    Ok(Some(l)) => l,
                    Ok(None) => {
                        let (st, why) = if cfg.mode == BridgeMode::Bundled {
                            (
                                SupervisorState::Error,
                                "Another bridge already holds this store (lock held); stop it or set the bridge mode to Auto or External.",
                            )
                        } else {
                            (
                                SupervisorState::External,
                                "Another wa-desk or bridge holds the store lock.",
                            )
                        };
                        self.set(st, None, Some(why.to_string()));
                        return;
                    }
                    Err(e) => {
                        self.set(
                            SupervisorState::Error,
                            None,
                            Some(format!("Cannot take the store lock: {e}")),
                        );
                        return;
                    }
                };
                return self
                    .supervise(cfg, port, spawner, probe, on_issue, lock)
                    .await;
            }
            Decision::External => (SupervisorState::External, None),
            Decision::Disabled if cfg.mode == BridgeMode::External => {
                (SupervisorState::Disabled, None)
            }
            Decision::Disabled => (
                SupervisorState::Disabled,
                Some(
                    "The bridge URL is not on this computer, so wa-desk will not start one."
                        .to_string(),
                ),
            ),
            Decision::NotInstalled => (SupervisorState::NotInstalled, None),
            Decision::PortBusy => (
                SupervisorState::Error,
                Some(format!(
                    "Port {port} is already in use by another program; stop it or set the bridge mode to Auto or External."
                )),
            ),
        };
        self.set(state, None, detail);
    }

    async fn supervise(
        &self,
        cfg: Config,
        port: u16,
        spawner: Arc<dyn Spawner>,
        probe: Arc<dyn Probe>,
        on_issue: IssueFn,
        lock: StoreLock,
    ) {
        let Some(binary) = cfg.binary.clone() else {
            return;
        };
        let cwd = cfg
            .bridge_store
            .parent()
            .map_or_else(|| cfg.bridge_store.clone(), Path::to_path_buf);
        let spec = SpawnSpec {
            program: binary,
            args: Vec::new(),
            cwd,
            env: vec![
                ("WHATSAPP_BRIDGE_PORT".into(), port.to_string()),
                ("WEBHOOK_ENABLED".into(), "false".into()),
                ("WHATSAPP_DEVICE_NAME".into(), "wa-desk".into()),
            ],
        };
        let log = Arc::new(RotatingLog::new(cfg.logs.join(LOG_NAME), LOG_CAP));
        let mut policy = RestartPolicy::default();
        let lock_file = lock_path(&cfg.bridge_store);
        let mut lock = Some(lock);
        loop {
            let started = Instant::now();
            let outcome = match ensure_private_dir(&spec.cwd) {
                Err(e) => Err(format!("cannot create {}: {e}", spec.cwd.display())),
                Ok(()) => spawner
                    .spawn(&spec, log.clone())
                    .map_err(|e| format!("could not start the bridge: {e}")),
            };
            let reason = match outcome {
                Err(e) => e,
                Ok(mut child) => {
                    tracing::info!(pid = child.pid(), "bundled bridge started");
                    self.set(SupervisorState::Running, child.pid(), None);
                    tokio::select! {
                        () = self.cancel.cancelled() => {
                            child.terminate(STOP_GRACE).await;
                            self.set(SupervisorState::Stopped, None, None);
                            return;
                        }
                        r = child.wait() => r,
                    }
                }
            };
            tracing::warn!(reason, "bundled bridge is not running");
            let Some(delay) = policy.on_exit(started.elapsed()) else {
                let detail = format!("{reason}. See {LOG_NAME} in the logs folder.");
                self.set(SupervisorState::GaveUp, None, Some(detail.clone()));
                on_issue("The WhatsApp bridge keeps crashing", &detail);
                return;
            };
            drop(lock.take());
            self.status.lock().restarts += 1;
            self.set(SupervisorState::Crashed, None, Some(reason));
            let delay = (*self.delays.lock()).unwrap_or(delay);
            tokio::select! {
                () = self.cancel.cancelled() => {
                    self.set(SupervisorState::Stopped, None, None);
                    return;
                }
                () = tokio::time::sleep(delay) => {}
            }
            // Someone else may have taken over while we waited: never run two
            // bridges on one store.
            if probe.is_up().await {
                self.set(
                    SupervisorState::External,
                    None,
                    Some("Another bridge now answers on the bridge port.".into()),
                );
                return;
            }
            match StoreLock::try_acquire(&lock_file) {
                Ok(Some(l)) => lock = Some(l),
                _ => {
                    self.set(
                        SupervisorState::External,
                        None,
                        Some("Another process now holds the store lock.".into()),
                    );
                    return;
                }
            }
        }
    }
}
