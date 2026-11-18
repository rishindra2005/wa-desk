//! Bridge supervisor: startup decision, backoff policy, restart loop with a
//! fake process, and (unix) a real spawn of a fake "bridge" script.
//! Nothing here touches a real bridge or a real store.

use std::collections::VecDeque;
use std::io;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use parking_lot::Mutex;
use wa_core::types::{BridgeMode, SupervisorState};
use wa_desk_lib::bridge_supervisor::{
    Config, Decision, Probe, RestartPolicy, RotatingLog, RunningChild, SpawnSpec, Spawner,
    Supervisor, decide, parse_bridge_url,
};

// ---------------------------------------------------------------- decision

#[test]
fn external_bridge_reachable_means_no_spawn() {
    assert_eq!(
        decide(BridgeMode::Auto, true, true, true),
        Decision::External
    );
}

#[test]
fn auto_without_bridge_spawns_when_installed_else_not_installed() {
    assert_eq!(decide(BridgeMode::Auto, true, false, true), Decision::Spawn);
    assert_eq!(
        decide(BridgeMode::Auto, true, false, false),
        Decision::NotInstalled
    );
}

#[test]
fn external_mode_never_spawns() {
    for reachable in [true, false] {
        for bin in [true, false] {
            assert_eq!(
                decide(BridgeMode::External, true, reachable, bin),
                Decision::Disabled
            );
        }
    }
}

#[test]
fn bundled_mode_spawns_but_reports_a_busy_port() {
    assert_eq!(
        decide(BridgeMode::Bundled, true, false, true),
        Decision::Spawn
    );
    assert_eq!(
        decide(BridgeMode::Bundled, true, true, true),
        Decision::PortBusy
    );
    assert_eq!(
        decide(BridgeMode::Bundled, true, false, false),
        Decision::NotInstalled
    );
}

#[test]
fn a_remote_bridge_url_is_never_spawned_for() {
    assert_eq!(
        decide(BridgeMode::Bundled, false, false, true),
        Decision::Disabled
    );
}

#[test]
fn bridge_url_parsing() {
    assert_eq!(
        parse_bridge_url("http://127.0.0.1:8080"),
        Some(("127.0.0.1".into(), 8080, true))
    );
    assert_eq!(
        parse_bridge_url("http://localhost:9000/"),
        Some(("localhost".into(), 9000, true))
    );
    assert_eq!(
        parse_bridge_url("http://192.168.1.9:8080"),
        Some(("192.168.1.9".into(), 8080, false))
    );
    assert_eq!(parse_bridge_url("nonsense"), None);
}

// ------------------------------------------------------------------ policy

#[test]
fn backoff_grows_and_is_capped_then_gives_up() {
    let mut p = RestartPolicy::default();
    let fast = Duration::from_millis(200);
    let secs = |s| Some(Duration::from_secs(s));
    assert_eq!(p.on_exit(fast), secs(1));
    assert_eq!(p.on_exit(fast), secs(2));
    assert_eq!(p.on_exit(fast), secs(5));
    assert_eq!(p.on_exit(fast), secs(10));
    assert_eq!(p.on_exit(fast), secs(20));
    assert_eq!(p.on_exit(fast), secs(40));
    assert_eq!(p.on_exit(fast), secs(60));
    assert_eq!(p.on_exit(fast), secs(60), "capped at 60 s");
    assert_eq!(
        p.on_exit(fast),
        None,
        "gives up after repeated fast crashes"
    );
}

#[test]
fn a_long_healthy_run_resets_the_backoff() {
    let mut p = RestartPolicy::default();
    for _ in 0..5 {
        p.on_exit(Duration::from_secs(1));
    }
    assert_eq!(
        p.on_exit(Duration::from_secs(3600)),
        Some(Duration::from_secs(1))
    );
    assert_eq!(
        p.on_exit(Duration::from_secs(1)),
        Some(Duration::from_secs(2))
    );
}

// -------------------------------------------------------------------- fakes

struct FakeProbe(bool);

#[async_trait::async_trait]
impl Probe for FakeProbe {
    async fn is_up(&self) -> bool {
        self.0
    }
}

/// Scripted child: exits after `run_for`, or never if `None`.
struct FakeChild {
    run_for: Option<Duration>,
    killed: Arc<Mutex<u32>>,
}

#[async_trait::async_trait]
impl RunningChild for FakeChild {
    fn pid(&self) -> Option<u32> {
        Some(4242)
    }
    async fn wait(&mut self) -> String {
        match self.run_for {
            Some(d) => {
                tokio::time::sleep(d).await;
                "exited with exit status: 1".into()
            }
            None => std::future::pending().await,
        }
    }
    async fn terminate(&mut self, _grace: Duration) {
        *self.killed.lock() += 1;
    }
}

#[derive(Default)]
struct FakeSpawner {
    /// Per spawn: `Err` = spawn fails, `Ok(run_for)`.
    script: Mutex<VecDeque<Result<Option<Duration>, String>>>,
    specs: Mutex<Vec<SpawnSpec>>,
    killed: Arc<Mutex<u32>>,
}

impl Spawner for FakeSpawner {
    fn spawn(&self, spec: &SpawnSpec, _log: Arc<RotatingLog>) -> io::Result<Box<dyn RunningChild>> {
        self.specs.lock().push(spec.clone());
        let next = self.script.lock().pop_front().unwrap_or(Ok(None));
        match next {
            Ok(run_for) => Ok(Box::new(FakeChild {
                run_for,
                killed: self.killed.clone(),
            })),
            Err(e) => Err(io::Error::other(e)),
        }
    }
}

struct Env {
    dir: tempfile::TempDir,
    sup: Arc<Supervisor>,
    spawner: Arc<FakeSpawner>,
    issues: Arc<Mutex<Vec<(String, String)>>>,
}

fn cfg(dir: &tempfile::TempDir, mode: BridgeMode, with_bin: bool) -> Config {
    let bin = dir.path().join("wa-bridge");
    if with_bin {
        std::fs::write(&bin, b"x").unwrap();
    }
    Config {
        mode,
        addr: Some(("127.0.0.1".into(), 8080, true)),
        binary: Some(bin),
        bridge_store: dir.path().join("runtime/store"),
        logs: dir.path().join("logs"),
    }
}

fn start(
    mode: BridgeMode,
    with_bin: bool,
    reachable: bool,
    script: Vec<Result<Option<Duration>, String>>,
) -> Env {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join("logs")).unwrap();
    let sup = Arc::new(Supervisor::new());
    sup.fixed_delay(Duration::from_millis(5));
    let spawner = Arc::new(FakeSpawner {
        script: Mutex::new(script.into()),
        ..Default::default()
    });
    let issues = Arc::new(Mutex::new(Vec::new()));
    let c = cfg(&dir, mode, with_bin);
    let (s, sp, is) = (sup.clone(), spawner.clone(), issues.clone());
    tokio::spawn(async move {
        s.run(
            c,
            sp,
            Arc::new(FakeProbe(reachable)),
            Arc::new(move |t, d| is.lock().push((t.to_string(), d.to_string()))),
        )
        .await;
    });
    Env {
        dir,
        sup,
        spawner,
        issues,
    }
}

async fn until(sup: &Supervisor, f: impl Fn(&wa_core::types::SupervisorStatus) -> bool) {
    for _ in 0..400 {
        if f(&sup.status()) {
            return;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("timed out; status {:?}", sup.status());
}

// -------------------------------------------------------------- supervisor

#[tokio::test]
async fn external_bridge_is_left_alone() {
    let e = start(BridgeMode::Auto, true, true, vec![]);
    until(&e.sup, |s| s.state == SupervisorState::External).await;
    assert!(e.spawner.specs.lock().is_empty());
}

#[tokio::test]
async fn missing_binary_is_not_installed() {
    let e = start(BridgeMode::Auto, false, false, vec![]);
    until(&e.sup, |s| s.state == SupervisorState::NotInstalled).await;
    assert!(e.spawner.specs.lock().is_empty());
}

#[tokio::test]
async fn external_mode_supervisor_never_spawns() {
    let e = start(BridgeMode::External, true, false, vec![]);
    until(&e.sup, |s| s.state == SupervisorState::Disabled).await;
    assert!(e.spawner.specs.lock().is_empty());
}

#[tokio::test]
async fn bundled_mode_with_busy_port_is_an_error() {
    let e = start(BridgeMode::Bundled, true, true, vec![]);
    until(&e.sup, |s| s.state == SupervisorState::Error).await;
    let detail = e.sup.status().detail.unwrap();
    assert!(
        detail.contains("8080") && detail.contains("in use"),
        "{detail}"
    );
    assert!(e.spawner.specs.lock().is_empty());
}

#[tokio::test]
async fn spawns_with_the_right_cwd_env_and_stops_cleanly() {
    let e = start(BridgeMode::Auto, true, false, vec![Ok(None)]);
    until(&e.sup, |s| s.state == SupervisorState::Running).await;
    assert_eq!(e.sup.status().pid, Some(4242));
    let spec = e.spawner.specs.lock()[0].clone();
    assert_eq!(spec.cwd, e.dir.path().join("runtime"));
    assert!(spec.cwd.is_dir(), "cwd is created");
    let env: std::collections::HashMap<_, _> = spec.env.into_iter().collect();
    assert_eq!(env["WHATSAPP_BRIDGE_PORT"], "8080");
    assert_eq!(env["WEBHOOK_ENABLED"], "false");
    assert_eq!(env["WHATSAPP_DEVICE_NAME"], "wa-desk");
    e.sup.stop().await;
    assert_eq!(*e.spawner.killed.lock(), 1);
    assert_eq!(e.sup.status().state, SupervisorState::Stopped);
}

#[tokio::test]
async fn restarts_after_an_unexpected_exit() {
    let e = start(
        BridgeMode::Auto,
        true,
        false,
        vec![Ok(Some(Duration::from_millis(5))), Ok(None)],
    );
    until(&e.sup, |s| {
        s.restarts == 1 && s.state == SupervisorState::Running
    })
    .await;
    assert_eq!(e.spawner.specs.lock().len(), 2);
    e.sup.stop().await;
}

#[tokio::test]
async fn spawn_failures_back_off_then_give_up_and_raise_an_issue() {
    let script = (0..20).map(|_| Err("boom".to_string())).collect();
    let e = start(BridgeMode::Auto, true, false, script);
    until(&e.sup, |s| s.state == SupervisorState::GaveUp).await;
    let st = e.sup.status();
    assert_eq!(st.restarts, 8);
    assert!(st.detail.unwrap().contains("boom"));
    assert_eq!(e.spawner.specs.lock().len(), 9);
    let issues = e.issues.lock();
    assert_eq!(issues.len(), 1);
    assert!(issues[0].0.contains("crashing"));
}

#[tokio::test]
async fn stop_during_backoff_does_not_respawn() {
    let e = start(BridgeMode::Auto, true, false, vec![Err("x".into())]);
    e.sup.fixed_delay(Duration::from_secs(30));
    until(&e.sup, |s| s.state == SupervisorState::Crashed).await;
    e.sup.stop().await;
    assert_eq!(e.sup.status().state, SupervisorState::Stopped);
    assert_eq!(e.spawner.specs.lock().len(), 1);
}

// --------------------------------------------------------------------- log

#[test]
fn log_rotates_at_the_cap() {
    let d = tempfile::tempdir().unwrap();
    let p: PathBuf = d.path().join("bridge.log");
    let log = RotatingLog::new(p.clone(), 100);
    for i in 0..30 {
        log.write_line(&format!("line number {i:04}")).unwrap();
    }
    let len = |p: &PathBuf| std::fs::metadata(p).unwrap().len();
    assert!(len(&p) <= 100);
    assert!(len(&d.path().join("bridge.log.1")) <= 100);
}

// ------------------------------------------------- real process (unix only)

#[cfg(unix)]
mod real {
    use super::*;
    use std::os::unix::fs::PermissionsExt;
    use wa_desk_lib::bridge_supervisor::TokioSpawner;

    #[tokio::test]
    async fn fake_bridge_script_gets_cwd_env_and_is_killed_by_stop() {
        let dir = tempfile::tempdir().unwrap();
        let bin = dir.path().join("wa-bridge");
        // Records its cwd and env, prints a token banner line, then idles.
        std::fs::write(
            &bin,
            "#!/bin/sh\npwd > seen-cwd.txt\nenv > seen-env.txt\necho hello-from-bridge\necho 'Token:  SECRET123'\nexec sleep 300\n",
        )
        .unwrap();
        std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o755)).unwrap();
        let logs = dir.path().join("logs");
        std::fs::create_dir_all(&logs).unwrap();
        let c = Config {
            mode: BridgeMode::Bundled,
            addr: Some(("127.0.0.1".into(), 18080, true)),
            binary: Some(bin),
            bridge_store: dir.path().join("data/runtime/store"),
            logs: logs.clone(),
        };
        let sup = Arc::new(Supervisor::new());
        let s = sup.clone();
        tokio::spawn(async move {
            s.run(
                c,
                Arc::new(TokioSpawner),
                Arc::new(FakeProbe(false)),
                Arc::new(|_, _| {}),
            )
            .await;
        });
        until(&sup, |s| s.state == SupervisorState::Running).await;
        let pid = sup.status().pid.expect("pid");
        let runtime = dir.path().join("data/runtime");
        let env_file = runtime.join("seen-env.txt");
        for _ in 0..300 {
            if env_file.exists() && logs.join("bridge.log").exists() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        let cwd = std::fs::read_to_string(runtime.join("seen-cwd.txt")).unwrap();
        assert_eq!(
            std::fs::canonicalize(cwd.trim()).unwrap(),
            std::fs::canonicalize(&runtime).unwrap()
        );
        let mode = std::fs::metadata(&runtime).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o700, "cwd is owner-only");
        let env = std::fs::read_to_string(env_file).unwrap();
        assert!(env.contains("WHATSAPP_BRIDGE_PORT=18080"));
        assert!(env.contains("WEBHOOK_ENABLED=false"));
        assert!(env.contains("WHATSAPP_DEVICE_NAME=wa-desk"));
        for _ in 0..300 {
            let t = std::fs::read_to_string(logs.join("bridge.log")).unwrap_or_default();
            if t.contains("hello-from-bridge") {
                assert!(!t.contains("SECRET123"), "token line must not be logged");
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        sup.stop().await;
        let gone = nix::sys::signal::kill(nix::unistd::Pid::from_raw(pid as i32), None);
        assert!(gone.is_err(), "process {pid} still alive after stop()");
        assert_eq!(sup.status().state, SupervisorState::Stopped);
    }
}
