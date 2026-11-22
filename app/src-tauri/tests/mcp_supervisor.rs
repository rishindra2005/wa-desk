//! MCP supervisor: uv and source discovery, the child's command line and
//! environment, apply (start / stop / restart), the terminal states, backoff,
//! and (unix) a real spawn of a fake `uv` script that listens on a port.
//! Nothing here touches a real MCP server, bridge or store.

use std::collections::VecDeque;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use parking_lot::Mutex;
use wa_core::config::Paths;
use wa_core::types::{McpState, Settings};
use wa_desk_lib::bridge_supervisor::{Clock, RotatingLog, RunningChild, SpawnSpec, Spawner};
// Only the unix test that runs a real fake `uv` needs the real clock.
#[cfg(unix)]
use wa_desk_lib::bridge_supervisor::TokioClock;
use wa_desk_lib::mcp_supervisor::{
    McpDeps, McpSupervisor, PortProbe, PortState, Timings, banner, build_args, build_env,
    build_spec, find_uv, resolve_source, uv_candidates, venv_dir,
};

// ------------------------------------------------------------- discovery

fn touch(p: &Path) {
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(p, b"x").unwrap();
}

#[test]
fn uv_candidates_are_path_then_local_bin_then_cargo_bin() {
    let path = std::env::join_paths(["/a", "/b"]).unwrap();
    let c = uv_candidates(Some(&path), Some(Path::new("/home/u")), false);
    assert_eq!(
        c,
        [
            "/a/uv",
            "/b/uv",
            "/home/u/.local/bin/uv",
            "/home/u/.cargo/bin/uv"
        ]
        .map(PathBuf::from)
    );
    let w = uv_candidates(None, Some(Path::new("/home/u")), true);
    assert_eq!(
        w,
        ["/home/u/.local/bin/uv.exe", "/home/u/.cargo/bin/uv.exe"].map(PathBuf::from)
    );
    assert!(uv_candidates(None, None, false).is_empty());
}

#[test]
fn find_uv_prefers_path_and_falls_back_to_home_folders() {
    let d = tempfile::tempdir().unwrap();
    let (bin, home) = (d.path().join("bin"), d.path().join("home"));
    let cands = |p: &Path| uv_candidates(Some(bin.as_os_str()), Some(p), false);
    assert_eq!(find_uv(&cands(&home)), None);

    touch(&home.join(".cargo/bin/uv"));
    assert_eq!(find_uv(&cands(&home)), Some(home.join(".cargo/bin/uv")));
    touch(&home.join(".local/bin/uv"));
    assert_eq!(find_uv(&cands(&home)), Some(home.join(".local/bin/uv")));
    touch(&bin.join("uv"));
    assert_eq!(find_uv(&cands(&home)), Some(bin.join("uv")));
}

fn make_source(dir: &Path) -> PathBuf {
    touch(&dir.join("main.py"));
    touch(&dir.join("pyproject.toml"));
    dir.to_path_buf()
}

#[test]
fn source_resolution_order_and_validity() {
    let d = tempfile::tempdir().unwrap();
    let env = make_source(&d.path().join("env"));
    let res_root = d.path().join("res");
    let res = make_source(&res_root.join("whatsapp-mcp-server"));
    let repo = make_source(&d.path().join("repo"));

    assert_eq!(
        resolve_source(Some(&env), Some(&res_root), Some(&repo)),
        Some(env.clone())
    );
    assert_eq!(
        resolve_source(None, Some(&res_root), Some(&repo)),
        Some(res.clone())
    );
    assert_eq!(resolve_source(None, None, Some(&repo)), Some(repo.clone()));
    // An override that is not a server folder falls through.
    let bad = d.path().join("empty");
    std::fs::create_dir_all(&bad).unwrap();
    assert_eq!(resolve_source(Some(&bad), Some(&res_root), None), Some(res));
    assert_eq!(resolve_source(Some(&bad), None, None), None);
    assert_eq!(resolve_source(None, None, None), None);
}

// ------------------------------------------------------- child command

fn paths(root: &Path) -> Paths {
    Paths::under_root(root, "http://127.0.0.1:8080".into())
}

#[test]
fn args_env_and_spec_for_the_child() {
    let root = Path::new("/r");
    let p = paths(root);
    let src = Path::new("/usr/lib/wa-desk/whatsapp-mcp-server");
    assert_eq!(
        build_args(src),
        [
            "run",
            "--project",
            "/usr/lib/wa-desk/whatsapp-mcp-server",
            "--frozen",
            "/usr/lib/wa-desk/whatsapp-mcp-server/main.py"
        ]
    );
    let env: std::collections::HashMap<_, _> = build_env(&p, 8771).into_iter().collect();
    assert_eq!(env["WHATSAPP_MCP_TRANSPORT"], "http");
    assert_eq!(env["WHATSAPP_MCP_HOST"], "127.0.0.1");
    assert_eq!(env["WHATSAPP_MCP_PORT"], "8771");
    assert_eq!(env["WHATSAPP_DB_PATH"], p.messages_db.display().to_string());
    assert_eq!(
        env["WHATSMEOW_DB_PATH"],
        p.whatsapp_db.display().to_string()
    );
    assert_eq!(env["WHATSAPP_API_URL"], "http://127.0.0.1:8080/api");
    assert_eq!(env["WHATSAPP_MEDIA_ROOTS"], p.outbox.display().to_string());
    assert_eq!(
        env["UV_PROJECT_ENVIRONMENT"],
        venv_dir(&p).display().to_string()
    );
    assert!(venv_dir(&p).starts_with(&p.cache));
    // The venv is never inside the (read-only) source folder.
    assert!(!venv_dir(&p).starts_with(src));

    let spec = build_spec(Path::new("/x/uv"), src, &p, 9000);
    assert_eq!(spec.program, Path::new("/x/uv"));
    assert!(spec.cwd.starts_with(&p.cache));
    assert!(
        spec.env
            .contains(&("WHATSAPP_MCP_PORT".into(), "9000".into()))
    );
}

#[test]
fn api_url_follows_the_bridge_url_without_double_slash() {
    let p = Paths::under_root(Path::new("/r"), "http://127.0.0.1:9090/".into());
    let env: std::collections::HashMap<_, _> = build_env(&p, 8771).into_iter().collect();
    assert_eq!(env["WHATSAPP_API_URL"], "http://127.0.0.1:9090/api");
}

#[test]
fn nothing_secret_is_passed_or_logged() {
    let p = paths(Path::new("/r"));
    let env = build_env(&p, 8771);
    assert!(env.iter().all(|(k, _)| !k.to_uppercase().contains("TOKEN")));
    let spec = build_spec(Path::new("/x/uv"), Path::new("/src"), &p, 8771);
    let line = banner(&spec);
    assert!(line.contains("WHATSAPP_MCP_PORT"), "names are logged");
    assert!(!line.contains("8771"), "values are not: {line}");
    assert!(!line.contains(&*p.bridge_store.to_string_lossy()));
    assert!(!line.to_lowercase().contains("token"));
}

// ----------------------------------------------------------------- fakes

/// Shared "what answers on the port" cell.
type PortCell = Arc<Mutex<PortState>>;

struct FakeProbe {
    cell: PortCell,
    /// Reports `Free` for this many polls after the cell turns up.
    delay_polls: Mutex<u32>,
}

#[async_trait::async_trait]
impl PortProbe for FakeProbe {
    async fn state(&self, _port: u16) -> PortState {
        let s = *self.cell.lock();
        if s != PortState::Free {
            let mut d = self.delay_polls.lock();
            if *d > 0 {
                *d -= 1;
                return PortState::Free;
            }
        }
        s
    }
}

struct FakeChild {
    run_for: Option<Duration>,
    killed: Arc<Mutex<u32>>,
    cell: PortCell,
}

#[async_trait::async_trait]
impl RunningChild for FakeChild {
    fn pid(&self) -> Option<u32> {
        Some(777)
    }
    async fn wait(&mut self) -> String {
        match self.run_for {
            Some(d) => {
                tokio::time::sleep(d).await;
                *self.cell.lock() = PortState::Free;
                "exited with exit status: 1".into()
            }
            None => std::future::pending().await,
        }
    }
    async fn terminate(&mut self, _grace: Duration) {
        *self.killed.lock() += 1;
        *self.cell.lock() = PortState::Free;
    }
}

struct FakeSpawner {
    /// Per spawn: `Err` = fails, `Ok(run_for)`; the default is "runs forever".
    script: Mutex<VecDeque<Result<Option<Duration>, String>>>,
    specs: Mutex<Vec<SpawnSpec>>,
    killed: Arc<Mutex<u32>>,
    cell: PortCell,
    /// Whether a spawned child opens the port.
    listens: bool,
}

impl Spawner for FakeSpawner {
    fn spawn(&self, spec: &SpawnSpec, _log: Arc<RotatingLog>) -> io::Result<Box<dyn RunningChild>> {
        self.specs.lock().push(spec.clone());
        let next = self.script.lock().pop_front().unwrap_or(Ok(None));
        match next {
            Ok(run_for) => {
                if self.listens {
                    *self.cell.lock() = PortState::Mcp;
                }
                Ok(Box::new(FakeChild {
                    run_for,
                    killed: self.killed.clone(),
                    cell: self.cell.clone(),
                }))
            }
            Err(e) => Err(io::Error::other(e)),
        }
    }
}

/// Records every sleep and yields so loops never starve the runtime. Sleeps
/// of 100 ms or more (the restart delays in the tests that set one) are real.
#[derive(Default)]
struct RecordingClock {
    sleeps: Mutex<Vec<Duration>>,
    real_long_sleeps: bool,
}

#[async_trait::async_trait]
impl Clock for RecordingClock {
    async fn sleep(&self, d: Duration) {
        self.sleeps.lock().push(d);
        if self.real_long_sleeps && d >= Duration::from_millis(100) {
            tokio::time::sleep(d).await;
        } else {
            tokio::task::yield_now().await;
        }
    }
}

struct Rig {
    dir: tempfile::TempDir,
    sup: McpSupervisor,
    spawner: Arc<FakeSpawner>,
    cell: PortCell,
    issues: Arc<Mutex<Vec<(String, String)>>>,
    clock: Arc<RecordingClock>,
}

struct Opts {
    uv: bool,
    source: bool,
    port_state: PortState,
    listens: bool,
    delay_polls: u32,
    script: Vec<Result<Option<Duration>, String>>,
    timings: Timings,
}

impl Default for Opts {
    fn default() -> Self {
        Self {
            uv: true,
            source: true,
            port_state: PortState::Free,
            listens: true,
            delay_polls: 0,
            script: Vec::new(),
            timings: Timings {
                first_start: Duration::from_secs(5),
                warm_start: Duration::from_secs(5),
                poll: Duration::from_millis(1),
                stop_grace: Duration::from_millis(50),
                fixed_delay: Some(Duration::from_millis(2)),
            },
        }
    }
}

fn rig(o: Opts) -> Rig {
    let dir = tempfile::tempdir().unwrap();
    let p = paths(dir.path());
    std::fs::create_dir_all(&p.logs).unwrap();
    let uv = dir.path().join("uv");
    let src = make_source(&dir.path().join("src"));
    let cell: PortCell = Arc::new(Mutex::new(o.port_state));
    let spawner = Arc::new(FakeSpawner {
        script: Mutex::new(o.script.into()),
        specs: Mutex::default(),
        killed: Arc::default(),
        cell: cell.clone(),
        listens: o.listens,
    });
    let issues = Arc::new(Mutex::new(Vec::new()));
    let is = issues.clone();
    let clock = Arc::new(RecordingClock {
        real_long_sleeps: o.timings.fixed_delay.is_some(),
        ..RecordingClock::default()
    });
    let sup = McpSupervisor::new(McpDeps {
        paths: p,
        find_uv: Arc::new(move || o.uv.then(|| uv.clone())),
        find_source: Arc::new(move || o.source.then(|| src.clone())),
        spawner: spawner.clone(),
        probe: Arc::new(FakeProbe {
            cell: cell.clone(),
            delay_polls: Mutex::new(o.delay_polls),
        }),
        clock: clock.clone(),
        on_issue: Arc::new(move |t, d| is.lock().push((t.into(), d.into()))),
        timings: o.timings,
    });
    Rig {
        dir,
        sup,
        spawner,
        cell,
        issues,
        clock,
    }
}

async fn until(what: &str, f: impl Fn() -> bool) {
    for _ in 0..1000 {
        if f() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    panic!("timed out waiting for: {what}");
}

async fn until_state(r: &Rig, s: McpState) {
    let mut last = r.sup.status().state;
    for _ in 0..1000 {
        last = r.sup.status().state;
        if last == s {
            return;
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    panic!("never reached {s:?}; last was {last:?}");
}

// --------------------------------------------------------------- apply

#[tokio::test]
async fn enabling_starts_the_child_with_the_right_command() {
    let r = rig(Opts::default());
    r.sup.apply(true, 8771).await;
    until_state(&r, McpState::Running).await;
    let st = r.sup.status();
    assert_eq!(st.pid, Some(777));
    assert_eq!(st.url, "http://127.0.0.1:8771/mcp");
    assert_eq!(st.restarts, 0);
    let specs = r.spawner.specs.lock();
    assert_eq!(specs.len(), 1);
    assert_eq!(specs[0].program, r.dir.path().join("uv"));
    assert_eq!(specs[0].args, build_args(&r.dir.path().join("src")));
    assert!(specs[0].cwd.starts_with(r.dir.path()));
    assert!(
        specs[0]
            .env
            .contains(&("WHATSAPP_MCP_PORT".into(), "8771".into()))
    );
}

#[tokio::test]
async fn applying_the_same_settings_again_does_nothing() {
    let r = rig(Opts::default());
    r.sup.apply(true, 8771).await;
    until_state(&r, McpState::Running).await;
    r.sup.apply(true, 8771).await;
    assert_eq!(r.spawner.specs.lock().len(), 1);
    assert_eq!(*r.spawner.killed.lock(), 0);
}

#[tokio::test]
async fn turning_it_off_stops_the_child() {
    let r = rig(Opts::default());
    r.sup.apply(true, 8771).await;
    until_state(&r, McpState::Running).await;
    r.sup.apply(false, 8771).await;
    assert_eq!(*r.spawner.killed.lock(), 1);
    assert_eq!(r.sup.status().state, McpState::Disabled);
    assert_eq!(r.sup.status().pid, None);
    // ... and back on starts a new one.
    r.sup.apply(true, 8771).await;
    until_state(&r, McpState::Running).await;
    assert_eq!(r.spawner.specs.lock().len(), 2);
}

#[tokio::test]
async fn changing_the_port_restarts_on_the_new_port() {
    let r = rig(Opts::default());
    r.sup.apply(true, 8771).await;
    until_state(&r, McpState::Running).await;
    r.sup.apply(true, 9001).await;
    until_state(&r, McpState::Running).await;
    assert_eq!(*r.spawner.killed.lock(), 1);
    let specs = r.spawner.specs.lock();
    assert_eq!(specs.len(), 2);
    assert!(
        specs[1]
            .env
            .contains(&("WHATSAPP_MCP_PORT".into(), "9001".into()))
    );
    assert_eq!(r.sup.status().url, "http://127.0.0.1:9001/mcp");
}

#[tokio::test]
async fn shutdown_stops_the_child_and_reports_stopped() {
    let r = rig(Opts::default());
    r.sup.apply(true, 8771).await;
    until_state(&r, McpState::Running).await;
    r.sup.shutdown().await;
    assert_eq!(*r.spawner.killed.lock(), 1);
    assert_eq!(r.sup.status().state, McpState::Stopped);
}

#[tokio::test]
async fn disabled_never_spawns() {
    let r = rig(Opts::default());
    r.sup.apply(false, 8771).await;
    assert_eq!(r.sup.status().state, McpState::Disabled);
    assert!(r.spawner.specs.lock().is_empty());
}

// ------------------------------------------------------- terminal states

#[tokio::test]
async fn missing_uv_says_needs_uv_with_a_hint_and_never_spawns() {
    let r = rig(Opts {
        uv: false,
        ..Opts::default()
    });
    r.sup.apply(true, 8771).await;
    until_state(&r, McpState::NeedsUv).await;
    let detail = r.sup.status().detail.unwrap();
    assert!(detail.contains("https://docs.astral.sh/uv/"), "{detail}");
    assert!(r.spawner.specs.lock().is_empty());
}

#[tokio::test]
async fn missing_source_says_not_installed() {
    let r = rig(Opts {
        source: false,
        ..Opts::default()
    });
    r.sup.apply(true, 8771).await;
    until_state(&r, McpState::NotInstalled).await;
    assert!(r.spawner.specs.lock().is_empty());
}

#[tokio::test]
async fn a_busy_port_that_is_not_mcp_says_port_busy() {
    let r = rig(Opts {
        port_state: PortState::Other,
        ..Opts::default()
    });
    r.sup.apply(true, 8771).await;
    until_state(&r, McpState::PortBusy).await;
    assert!(r.sup.status().detail.unwrap().contains("8771"));
    assert!(r.spawner.specs.lock().is_empty());
}

#[tokio::test]
async fn an_mcp_server_already_answering_is_external() {
    let r = rig(Opts {
        port_state: PortState::Mcp,
        ..Opts::default()
    });
    r.sup.apply(true, 8771).await;
    until_state(&r, McpState::External).await;
    assert!(r.spawner.specs.lock().is_empty());
}

#[tokio::test]
async fn an_ended_state_is_retried_by_the_next_apply() {
    let r = rig(Opts {
        port_state: PortState::Other,
        ..Opts::default()
    });
    r.sup.apply(true, 8771).await;
    until_state(&r, McpState::PortBusy).await;
    *r.cell.lock() = PortState::Free;
    r.sup.apply(true, 8771).await;
    until_state(&r, McpState::Running).await;
}

// ------------------------------------------------- backoff, give-up, slow

#[tokio::test]
async fn crashes_restart_with_growing_delays_then_give_up_with_an_issue() {
    let r = rig(Opts {
        // Every spawn exits at once. Real (not fixed) delays are recorded.
        script: (0..20).map(|_| Ok(Some(Duration::ZERO))).collect(),
        timings: Timings {
            fixed_delay: None,
            ..Opts::default().timings
        },
        ..Opts::default()
    });
    r.sup.apply(true, 8771).await;
    until_state(&r, McpState::GaveUp).await;
    let st = r.sup.status();
    assert_eq!(st.restarts, 8);
    assert_eq!(r.spawner.specs.lock().len(), 9);
    assert!(st.detail.unwrap().contains("mcp.log"));
    let issues = r.issues.lock();
    assert_eq!(issues.len(), 1);
    assert!(issues[0].0.contains("MCP server"));

    let delays: Vec<u64> = r
        .clock
        .sleeps
        .lock()
        .iter()
        .filter(|d| **d >= Duration::from_secs(1))
        .map(Duration::as_secs)
        .collect();
    assert_eq!(delays, [1, 2, 5, 10, 20, 40, 60, 60]);
}

#[tokio::test]
async fn a_failed_spawn_counts_as_a_crash() {
    let r = rig(Opts {
        script: vec![Err("no such file".into()), Ok(None)],
        ..Opts::default()
    });
    r.sup.apply(true, 8771).await;
    until_state(&r, McpState::Running).await;
    assert_eq!(r.sup.status().restarts, 1);
}

#[tokio::test]
async fn a_crash_while_someone_else_took_the_port_becomes_external() {
    let r = rig(Opts {
        script: vec![Ok(Some(Duration::from_millis(30)))],
        timings: Timings {
            fixed_delay: Some(Duration::from_millis(400)),
            ..Opts::default().timings
        },
        ..Opts::default()
    });
    r.sup.apply(true, 8771).await;
    until_state(&r, McpState::Running).await;
    until_state(&r, McpState::Crashed).await;
    *r.cell.lock() = PortState::Mcp;
    until_state(&r, McpState::External).await;
    assert_eq!(r.spawner.specs.lock().len(), 1);
}

#[tokio::test]
async fn a_slow_first_start_is_not_a_crash() {
    // The child stays alive but the port does not answer yet.
    let r = rig(Opts {
        listens: false,
        ..Opts::default()
    });
    r.sup.apply(true, 8771).await;
    until("a starting state with the first-run hint", || {
        let s = r.sup.status();
        s.state == McpState::Starting
            && s.pid == Some(777)
            && s.detail.as_deref().is_some_and(|d| d.contains("first run"))
    })
    .await;
    *r.cell.lock() = PortState::Mcp; // the server finally listens
    until_state(&r, McpState::Running).await;
    assert_eq!(r.sup.status().restarts, 0);
    assert_eq!(r.spawner.specs.lock().len(), 1);
}

#[tokio::test]
async fn a_start_that_never_answers_is_stopped_and_retried() {
    let r = rig(Opts {
        listens: false,
        timings: Timings {
            first_start: Duration::from_millis(60),
            warm_start: Duration::from_millis(60),
            ..Opts::default().timings
        },
        ..Opts::default()
    });
    r.sup.apply(true, 8771).await;
    until("a timed-out start", || *r.spawner.killed.lock() >= 1).await;
    until("a restart", || r.sup.status().restarts >= 1).await;
    r.sup.apply(false, 8771).await;
    assert_eq!(r.sup.status().state, McpState::Disabled);
}

#[test]
fn defaults_give_a_first_start_plenty_of_time() {
    let t = Timings::default();
    assert!(t.first_start >= Duration::from_secs(10 * 60));
    assert!(t.first_start > t.warm_start);
}

#[test]
fn settings_defaults_are_on_and_8771() {
    let s = Settings::default();
    assert!(s.mcp_enabled);
    assert_eq!(s.mcp_port, 8771);
    assert_eq!(Settings::mcp_url(s.mcp_port), "http://127.0.0.1:8771/mcp");
}

// -------------------------------------------------- real process (unix)

#[cfg(unix)]
mod real {
    use super::*;
    use std::os::unix::fs::PermissionsExt;
    use wa_desk_lib::bridge_supervisor::TokioSpawner;
    use wa_desk_lib::mcp_supervisor::HttpPortProbe;

    fn have_python() -> bool {
        std::process::Command::new("python3")
            .arg("--version")
            .output()
            .is_ok_and(|o| o.status.success())
    }

    fn free_port() -> u16 {
        let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        l.local_addr().unwrap().port()
    }

    /// A fake `uv`: records how it was called, then listens on the port from
    /// the environment and answers any request with a JSON-RPC error body, like
    /// the real server does for a bare GET.
    const FAKE_UV: &str = r#"#!/bin/sh
pwd > seen-cwd.txt
echo "$@" > seen-args.txt
env > seen-env.txt
echo hello-from-uv
echo 'Token:  SECRET123'
exec python3 -c '
import http.server, os
class H(http.server.BaseHTTPRequestHandler):
    def do_GET(self):
        b = b"{\"jsonrpc\":\"2.0\",\"id\":\"server-error\",\"error\":{\"code\":-32600,\"message\":\"x\"}}"
        self.send_response(400)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(b)))
        self.end_headers()
        self.wfile.write(b)
    def log_message(self, *a): pass
http.server.HTTPServer(("127.0.0.1", int(os.environ["WHATSAPP_MCP_PORT"])), H).serve_forever()
'
"#;

    #[tokio::test]
    async fn readiness_is_detected_by_polling_the_http_port_and_stop_frees_it() {
        if !have_python() {
            eprintln!("python3 not found; skipping");
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        let p = paths(dir.path());
        std::fs::create_dir_all(&p.logs).unwrap();
        let uv = dir.path().join("uv");
        std::fs::write(&uv, FAKE_UV).unwrap();
        std::fs::set_permissions(&uv, std::fs::Permissions::from_mode(0o755)).unwrap();
        let src = make_source(&dir.path().join("src"));
        let port = free_port();
        let sup = McpSupervisor::new(McpDeps {
            paths: p.clone(),
            find_uv: Arc::new({
                let uv = uv.clone();
                move || Some(uv.clone())
            }),
            find_source: Arc::new({
                let src = src.clone();
                move || Some(src.clone())
            }),
            spawner: Arc::new(TokioSpawner),
            probe: Arc::new(HttpPortProbe),
            clock: Arc::new(TokioClock),
            on_issue: Arc::new(|_, _| {}),
            timings: Timings {
                poll: Duration::from_millis(50),
                ..Timings::default()
            },
        });
        sup.apply(true, port).await;
        for _ in 0..400 {
            if sup.status().state == McpState::Running {
                break;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        let st = sup.status();
        assert_eq!(st.state, McpState::Running, "{st:?}");
        assert!(st.pid.is_some());
        assert_eq!(st.url, format!("http://127.0.0.1:{port}/mcp"));
        // A second look at the port sees an MCP server.
        assert_eq!(HttpPortProbe.state(port).await, PortState::Mcp);

        let work = wa_desk_lib::mcp_supervisor::work_dir(&p);
        let args = std::fs::read_to_string(work.join("seen-args.txt")).unwrap();
        assert_eq!(args.trim(), build_args(&src).join(" "));
        let env = std::fs::read_to_string(work.join("seen-env.txt")).unwrap();
        assert!(env.contains(&format!("WHATSAPP_MCP_PORT={port}")));
        assert!(env.contains("WHATSAPP_MCP_TRANSPORT=http"));
        assert!(env.contains("UV_PROJECT_ENVIRONMENT="));

        sup.apply(false, port).await;
        assert_eq!(sup.status().state, McpState::Disabled);
        assert_eq!(HttpPortProbe.state(port).await, PortState::Free);

        let log = std::fs::read_to_string(p.logs.join("mcp.log")).unwrap();
        assert!(log.contains("hello-from-uv"));
        assert!(log.contains("wa-desk: starting"));
        assert!(!log.contains("SECRET123"), "token line leaked: {log}");
    }

    #[tokio::test]
    async fn a_listener_that_is_not_mcp_counts_as_port_busy() {
        let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = l.local_addr().unwrap().port();
        std::thread::spawn(move || {
            use std::io::Write;
            for s in l.incoming().flatten() {
                let mut s = s;
                let _ = s.write_all(b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\n\r\n");
            }
        });
        assert_eq!(HttpPortProbe.state(port).await, PortState::Other);
    }
}
