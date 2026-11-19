//! Runs the bundled MCP server (the Python server in `bridge/whatsapp-mcp-server`)
//! when `Settings.mcp_enabled` is on.
//!
//! It follows the bridge supervisor's patterns and reuses its pieces
//! ([`Spawner`], [`RunningChild`], [`RotatingLog`], [`RestartPolicy`]):
//!
//! - pure helpers find `uv` ([`uv_candidates`], [`find_uv`]) and the server
//!   source ([`resolve_source`]), and build the child's command line
//!   ([`build_args`], [`build_env`]);
//! - [`McpSupervisor`] applies the settings at runtime ([`McpSupervisor::apply`]
//!   starts, stops or restarts the child) and publishes an
//!   [`McpStatus`]; the process, port probe and clock are traits so the whole
//!   loop is testable with fakes.
//!
//! The server only listens on `127.0.0.1`. It reads the same stores as the
//! bridge (read-only) and gets the bridge token from the `.bridge-token` file
//! next to `whatsapp.db`, never from our environment or logs.

use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use parking_lot::Mutex;
use tokio_util::sync::CancellationToken;
use wa_core::config::Paths;
use wa_core::types::{McpState, McpStatus, Settings};

use crate::bridge_supervisor::{
    Clock, IssueFn, RestartPolicy, RotatingLog, RunningChild, STOP_GRACE, SpawnSpec, Spawner,
    TokioClock, ensure_private_dir,
};

/// Dev override for the server source directory.
pub const DIR_ENV: &str = "WA_DESK_MCP_DIR";
/// Name of the server's directory in the Tauri resources and in the repo.
pub const SOURCE_DIR_NAME: &str = "whatsapp-mcp-server";
/// File name of the server's output log inside the app logs folder.
pub const LOG_NAME: &str = "mcp.log";
/// Size at which `mcp.log` rotates.
pub const LOG_CAP: u64 = 5 * 1024 * 1024;
/// Where to get `uv`.
pub const UV_HINT: &str = "Install uv from https://docs.astral.sh/uv/ to run the MCP server.";
/// Shown while the first run builds its Python environment.
pub const FIRST_RUN_HINT: &str = "Starting (first run installs dependencies)";

// ----------------------------------------------------------------- locate

/// Where `uv` may be, in lookup order: every `PATH` entry, then the usual
/// per-user install folders (desktop launches often lack `~/.local/bin`).
pub fn uv_candidates(path_var: Option<&OsStr>, home: Option<&Path>, windows: bool) -> Vec<PathBuf> {
    let exe = if windows { "uv.exe" } else { "uv" };
    let mut out: Vec<PathBuf> = path_var
        .map(|p| std::env::split_paths(p).map(|d| d.join(exe)).collect())
        .unwrap_or_default();
    if let Some(h) = home {
        out.push(h.join(".local").join("bin").join(exe));
        out.push(h.join(".cargo").join("bin").join(exe));
    }
    out
}

/// First candidate that is a file.
pub fn find_uv(candidates: &[PathBuf]) -> Option<PathBuf> {
    candidates.iter().find(|c| c.is_file()).cloned()
}

/// Look for `uv` on this machine (`PATH`, then the home folders).
pub fn find_uv_on_system() -> Option<PathBuf> {
    let home = if cfg!(windows) {
        std::env::var_os("USERPROFILE")
            .filter(|v| !v.is_empty())
            .map(PathBuf::from)
    } else {
        None
    }
    .or_else(dirs::home_dir);
    let path = std::env::var_os("PATH");
    find_uv(&uv_candidates(
        path.as_deref(),
        home.as_deref(),
        cfg!(windows),
    ))
}

fn is_source(dir: &Path) -> bool {
    dir.join("main.py").is_file() && dir.join("pyproject.toml").is_file()
}

/// Pick the server source: the dev override, then the bundled resources, then
/// (debug builds only) the repo checkout. `None` means "not installed".
pub fn resolve_source(
    env_dir: Option<&Path>,
    resource_dir: Option<&Path>,
    repo_dir: Option<&Path>,
) -> Option<PathBuf> {
    env_dir
        .map(Path::to_path_buf)
        .into_iter()
        .chain(resource_dir.map(|r| r.join(SOURCE_DIR_NAME)))
        .chain(repo_dir.map(Path::to_path_buf))
        .find(|d| is_source(d))
}

/// The repo's copy of the server, for `cargo tauri dev` (debug builds only, so
/// a release binary never looks at its build machine's paths).
pub fn repo_source_dir() -> Option<PathBuf> {
    if !cfg!(debug_assertions) {
        return None;
    }
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../bridge")
        .join(SOURCE_DIR_NAME)
        .canonicalize()
        .ok()
}

// ------------------------------------------------------------- child spec

/// Folder holding the Python environment, in the (writable) app cache.
pub fn venv_dir(paths: &Paths) -> PathBuf {
    paths.cache.join("mcp-venv")
}

/// Working directory of the child (the resource folder is read-only).
pub fn work_dir(paths: &Paths) -> PathBuf {
    paths.cache.join("mcp")
}

/// `uv` arguments: the project is `source`, resolved from its committed lock
/// file only (`--frozen`), and `main.py` is given by absolute path because the
/// working directory is not the project.
pub fn build_args(source: &Path) -> Vec<String> {
    vec![
        "run".into(),
        "--project".into(),
        source.display().to_string(),
        "--frozen".into(),
        source.join("main.py").display().to_string(),
    ]
}

/// Environment for the child. There is deliberately no token here: the server
/// reads `.bridge-token` next to `WHATSMEOW_DB_PATH` itself.
pub fn build_env(paths: &Paths, port: u16) -> Vec<(String, String)> {
    let api = format!("{}/api", paths.bridge_url.trim_end_matches('/'));
    vec![
        ("WHATSAPP_MCP_TRANSPORT".into(), "http".into()),
        ("WHATSAPP_MCP_HOST".into(), "127.0.0.1".into()),
        ("WHATSAPP_MCP_PORT".into(), port.to_string()),
        (
            "WHATSAPP_DB_PATH".into(),
            paths.messages_db.display().to_string(),
        ),
        (
            "WHATSMEOW_DB_PATH".into(),
            paths.whatsapp_db.display().to_string(),
        ),
        ("WHATSAPP_API_URL".into(), api),
        (
            "WHATSAPP_MEDIA_ROOTS".into(),
            paths.outbox.display().to_string(),
        ),
        (
            "UV_PROJECT_ENVIRONMENT".into(),
            venv_dir(paths).display().to_string(),
        ),
        ("UV_NO_PROGRESS".into(), "1".into()),
        ("PYTHONUNBUFFERED".into(), "1".into()),
        // The source folder may be read-only.
        ("PYTHONDONTWRITEBYTECODE".into(), "1".into()),
    ]
}

/// The full spawn request.
pub fn build_spec(uv: &Path, source: &Path, paths: &Paths, port: u16) -> SpawnSpec {
    SpawnSpec {
        program: uv.to_path_buf(),
        args: build_args(source),
        cwd: work_dir(paths),
        env: build_env(paths, port),
    }
}

/// The one line written to `mcp.log` before each start: the command and the
/// environment variable *names* only.
pub fn banner(spec: &SpawnSpec) -> String {
    let keys: Vec<&str> = spec.env.iter().map(|(k, _)| k.as_str()).collect();
    format!(
        "wa-desk: starting {} {} (env: {})",
        spec.program.display(),
        spec.args.join(" "),
        keys.join(", ")
    )
}

// ------------------------------------------------------------------ probe

/// What answers on the MCP port.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PortState {
    /// Nothing accepts connections.
    Free,
    /// An MCP server answers.
    Mcp,
    /// Something else holds the port.
    Other,
}

/// Looks at the MCP port.
#[async_trait::async_trait]
pub trait PortProbe: Send + Sync {
    /// Who, if anyone, answers on `127.0.0.1:port`.
    async fn state(&self, port: u16) -> PortState;
}

/// Real probe: a TCP connect, then `GET /mcp` and a look for a JSON-RPC body
/// (the MCP server answers a bare GET with a JSON-RPC error).
pub struct HttpPortProbe;

#[async_trait::async_trait]
impl PortProbe for HttpPortProbe {
    async fn state(&self, port: u16) -> PortState {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let c = tokio::net::TcpStream::connect(("127.0.0.1", port));
        let Ok(Ok(mut s)) = tokio::time::timeout(Duration::from_millis(800), c).await else {
            return PortState::Free;
        };
        let req =
            format!("GET /mcp HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nConnection: close\r\n\r\n");
        // Read until the JSON-RPC marker shows up, the peer closes, or 4 KiB.
        let exchange = async {
            s.write_all(req.as_bytes()).await?;
            let mut got = Vec::new();
            let mut buf = [0u8; 1024];
            while got.len() < 4096 {
                let n = s.read(&mut buf).await?;
                if n == 0 {
                    break;
                }
                got.extend_from_slice(&buf[..n]);
                if String::from_utf8_lossy(&got).contains("jsonrpc") {
                    break;
                }
            }
            Ok::<_, std::io::Error>(got)
        };
        match tokio::time::timeout(Duration::from_millis(1500), exchange).await {
            Ok(Ok(b)) if String::from_utf8_lossy(&b).contains("jsonrpc") => PortState::Mcp,
            _ => PortState::Other,
        }
    }
}

// ------------------------------------------------------------ supervisor

/// Timings; the defaults are for the app, tests shrink them.
#[derive(Debug, Clone)]
pub struct Timings {
    /// How long a start may take while the Python environment does not exist
    /// yet (the first run downloads and installs packages).
    pub first_start: Duration,
    /// How long a start may take when the environment already exists.
    pub warm_start: Duration,
    /// Gap between readiness polls.
    pub poll: Duration,
    /// Grace for the child to exit when asked to stop.
    pub stop_grace: Duration,
    /// Replaces every restart delay (tests).
    pub fixed_delay: Option<Duration>,
}

impl Default for Timings {
    fn default() -> Self {
        Self {
            first_start: Duration::from_secs(15 * 60),
            warm_start: Duration::from_secs(90),
            poll: Duration::from_millis(500),
            stop_grace: STOP_GRACE,
            fixed_delay: None,
        }
    }
}

/// Finds `uv` or the server source at start time.
pub type Locator = Arc<dyn Fn() -> Option<PathBuf> + Send + Sync>;

/// Everything outside the settings that the supervisor needs.
#[derive(Clone)]
pub struct McpDeps {
    /// Where the files live.
    pub paths: Paths,
    /// Finds `uv`.
    pub find_uv: Locator,
    /// Finds the server source directory.
    pub find_source: Locator,
    /// Starts processes.
    pub spawner: Arc<dyn Spawner>,
    /// Looks at the port.
    pub probe: Arc<dyn PortProbe>,
    /// Sleeps.
    pub clock: Arc<dyn Clock>,
    /// Records an issue when it gives up.
    pub on_issue: IssueFn,
    /// Timings.
    pub timings: Timings,
}

struct Slot {
    enabled: bool,
    port: u16,
    cancel: CancellationToken,
    task: tokio::task::JoinHandle<()>,
}

struct Inner {
    deps: McpDeps,
    status: Mutex<McpStatus>,
}

impl Inner {
    fn set(&self, state: McpState, pid: Option<u32>, detail: Option<String>) {
        let mut s = self.status.lock();
        s.state = state;
        s.pid = pid;
        s.detail = detail;
    }
}

/// Owns the MCP server process and publishes its state.
pub struct McpSupervisor {
    inner: Arc<Inner>,
    slot: tokio::sync::Mutex<Option<Slot>>,
}

impl McpSupervisor {
    /// A supervisor that runs nothing until [`Self::apply`].
    pub fn new(deps: McpDeps) -> Self {
        let status = McpStatus {
            state: McpState::Disabled,
            pid: None,
            url: Settings::mcp_url(Settings::DEFAULT_MCP_PORT),
            detail: None,
            restarts: 0,
        };
        Self {
            inner: Arc::new(Inner {
                deps,
                status: Mutex::new(status),
            }),
            slot: tokio::sync::Mutex::new(None),
        }
    }

    /// The app's supervisor (real `uv` lookup, process spawner and probe).
    pub fn for_app(paths: Paths, resource_dir: Option<PathBuf>, on_issue: IssueFn) -> Self {
        let find_source: Locator = Arc::new(move || {
            let env = std::env::var_os(DIR_ENV)
                .filter(|v| !v.is_empty())
                .map(PathBuf::from);
            resolve_source(
                env.as_deref(),
                resource_dir.as_deref(),
                repo_source_dir().as_deref(),
            )
        });
        Self::new(McpDeps {
            paths,
            find_uv: Arc::new(find_uv_on_system),
            find_source,
            spawner: Arc::new(crate::bridge_supervisor::TokioSpawner),
            probe: Arc::new(HttpPortProbe),
            clock: Arc::new(TokioClock),
            on_issue,
            timings: Timings::default(),
        })
    }

    /// Current state.
    pub fn status(&self) -> McpStatus {
        self.inner.status.lock().clone()
    }

    /// Make reality match the settings: start, stop or restart the child.
    /// Returns once a stop is complete or a start has been launched (it does
    /// not wait for the server to answer). A task that already ended (for
    /// example "needs uv" or "gave up") is retried.
    pub async fn apply(&self, enabled: bool, port: u16) {
        let mut slot = self.slot.lock().await;
        if let Some(cur) = slot.as_ref()
            && cur.enabled == enabled
            && cur.port == port
            && (!enabled || !cur.task.is_finished())
        {
            return;
        }
        if let Some(old) = slot.take() {
            self.stop_slot(old).await;
        }
        {
            let mut s = self.inner.status.lock();
            s.url = Settings::mcp_url(port);
            s.restarts = 0;
        }
        let cancel = CancellationToken::new();
        let task = if enabled {
            self.inner.set(McpState::Starting, None, None);
            let (inner, c) = (self.inner.clone(), cancel.clone());
            tokio::spawn(async move { run(inner, port, c).await })
        } else {
            self.inner.set(McpState::Disabled, None, None);
            tokio::spawn(async {})
        };
        *slot = Some(Slot {
            enabled,
            port,
            cancel,
            task,
        });
    }

    /// Stop the child and the loop (app exit); returns when it is gone.
    pub async fn shutdown(&self) {
        let mut slot = self.slot.lock().await;
        if let Some(old) = slot.take() {
            self.stop_slot(old).await;
        }
        self.inner.set(McpState::Stopped, None, None);
    }

    async fn stop_slot(&self, slot: Slot) {
        slot.cancel.cancel();
        let wait = self.inner.deps.timings.stop_grace * 2 + Duration::from_secs(1);
        let _ = tokio::time::timeout(wait, slot.task).await;
    }
}

fn first_run(paths: &Paths) -> bool {
    !venv_dir(paths).join("pyvenv.cfg").is_file()
}

/// Decide whether to spawn at all; `Err` carries the final state.
async fn preflight(inner: &Inner, port: u16) -> Result<(PathBuf, PathBuf), ()> {
    let d = &inner.deps;
    let Some(uv) = (d.find_uv)() else {
        inner.set(McpState::NeedsUv, None, Some(UV_HINT.into()));
        return Err(());
    };
    let Some(src) = (d.find_source)() else {
        inner.set(
            McpState::NotInstalled,
            None,
            Some("This install has no MCP server.".into()),
        );
        return Err(());
    };
    if !check_port(inner, port).await {
        return Err(());
    }
    Ok((uv, src))
}

/// True when the port is free; otherwise sets `external` / `port_busy`.
async fn check_port(inner: &Inner, port: u16) -> bool {
    match inner.deps.probe.state(port).await {
        PortState::Free => true,
        PortState::Mcp => {
            inner.set(
                McpState::External,
                None,
                Some(format!("An MCP server already answers on port {port}.")),
            );
            false
        }
        PortState::Other => {
            inner.set(
                McpState::PortBusy,
                None,
                Some(format!(
                    "Port {port} is in use by another program; pick another port."
                )),
            );
            false
        }
    }
}

enum Outcome {
    Cancelled,
    Exited(String),
}

async fn run(inner: Arc<Inner>, port: u16, cancel: CancellationToken) {
    let Ok((uv, src)) = preflight(&inner, port).await else {
        return;
    };
    let d = inner.deps.clone();
    let spec = build_spec(&uv, &src, &d.paths, port);
    let log = Arc::new(RotatingLog::new(d.paths.logs.join(LOG_NAME), LOG_CAP));
    let mut policy = RestartPolicy::default();
    loop {
        let started = Instant::now();
        let outcome = start_once(&inner, &spec, &log, port, &cancel).await;
        let reason = match outcome {
            Outcome::Cancelled => {
                inner.set(McpState::Stopped, None, None);
                return;
            }
            Outcome::Exited(r) => r,
        };
        tracing::warn!(reason, "MCP server is not running");
        let Some(delay) = policy.on_exit(started.elapsed()) else {
            let detail = format!("{reason}. See {LOG_NAME} in the logs folder.");
            inner.set(McpState::GaveUp, None, Some(detail.clone()));
            (d.on_issue)("The MCP server keeps crashing", &detail);
            return;
        };
        inner.status.lock().restarts += 1;
        inner.set(McpState::Crashed, None, Some(reason));
        let delay = d.timings.fixed_delay.unwrap_or(delay);
        tokio::select! {
            () = cancel.cancelled() => {
                inner.set(McpState::Stopped, None, None);
                return;
            }
            () = d.clock.sleep(delay) => {}
        }
        // Someone else may have taken the port while we waited.
        if !check_port(&inner, port).await {
            return;
        }
    }
}

/// Spawn the child, wait until it answers, then until it exits.
async fn start_once(
    inner: &Inner,
    spec: &SpawnSpec,
    log: &Arc<RotatingLog>,
    port: u16,
    cancel: &CancellationToken,
) -> Outcome {
    let d = &inner.deps;
    let first = first_run(&d.paths);
    let limit = if first {
        d.timings.first_start
    } else {
        d.timings.warm_start
    };
    let hint = if first { FIRST_RUN_HINT } else { "Starting" };
    inner.set(McpState::Starting, None, Some(hint.into()));
    if let Err(e) = ensure_private_dir(&spec.cwd) {
        return Outcome::Exited(format!("cannot create {}: {e}", spec.cwd.display()));
    }
    let _ = log.write_line(&banner(spec));
    let mut child = match d.spawner.spawn(spec, log.clone()) {
        Ok(c) => c,
        Err(e) => return Outcome::Exited(format!("could not start uv: {e}")),
    };
    let pid = child.pid();
    tracing::info!(pid, "MCP server process started");
    inner.set(McpState::Starting, pid, Some(hint.into()));
    let begun = Instant::now();
    let ready = async {
        loop {
            if d.probe.state(port).await != PortState::Free {
                return true;
            }
            if begun.elapsed() >= limit {
                return false;
            }
            d.clock.sleep(d.timings.poll).await;
        }
    };
    tokio::select! {
        () = cancel.cancelled() => return stop_child(child, d).await,
        r = child.wait() => return Outcome::Exited(r),
        ok = ready => if !ok {
            child.terminate(d.timings.stop_grace).await;
            return Outcome::Exited(format!(
                "the server did not answer within {} s",
                limit.as_secs()
            ));
        }
    }
    inner.set(McpState::Running, pid, None);
    tokio::select! {
        () = cancel.cancelled() => stop_child(child, d).await,
        r = child.wait() => Outcome::Exited(r),
    }
}

async fn stop_child(mut child: Box<dyn RunningChild>, d: &McpDeps) -> Outcome {
    child.terminate(d.timings.stop_grace).await;
    Outcome::Cancelled
}
