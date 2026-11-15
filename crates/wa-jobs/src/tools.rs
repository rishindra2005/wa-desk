//! Running external tools safely: own process group, timeout, cancellation
//! (SIGTERM to the group, SIGKILL after 3 s) and a friendly error when the
//! tool is not installed.

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::{ExitStatus, Stdio};
use std::time::Duration;

use nix::sys::signal::{Signal, killpg};
use nix::unistd::Pid;
use tokio::io::{AsyncRead, AsyncReadExt};
use tokio::process::Child;
use tokio_util::sync::CancellationToken;
use wa_core::{AppError, Result};

/// Grace period between SIGTERM and SIGKILL.
pub const KILL_GRACE: Duration = Duration::from_secs(3);
const STDOUT_CAP: usize = 1 << 20;
const STDERR_TAIL: usize = 4096;

/// Programs used by the jobs. Override for tests or unusual installs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Tools {
    pub ffmpeg: PathBuf,
    pub ffprobe: PathBuf,
    pub soffice: PathBuf,
    pub gs: PathBuf,
    pub pdftoppm: PathBuf,
    pub rsync: PathBuf,
}

impl Default for Tools {
    fn default() -> Self {
        Self {
            ffmpeg: "ffmpeg".into(),
            ffprobe: "ffprobe".into(),
            soffice: "soffice".into(),
            gs: "gs".into(),
            pdftoppm: "pdftoppm".into(),
            rsync: "rsync".into(),
        }
    }
}

/// Captured result of a finished tool run.
#[derive(Debug, Clone, Default)]
pub struct ToolOutput {
    pub code: Option<i32>,
    /// Stdout (capped at 1 MiB).
    pub stdout: String,
    /// Last 4 KiB of stderr.
    pub stderr_tail: String,
}

/// Builder for one external command.
pub struct Tool {
    name: String,
    program: PathBuf,
    args: Vec<OsString>,
    timeout: Duration,
    ok_codes: Vec<i32>,
}

impl Tool {
    /// `name` is the user-facing tool name used in errors ("ffmpeg").
    pub fn new(name: &str, program: &Path) -> Self {
        Self {
            name: name.to_string(),
            program: program.to_path_buf(),
            args: Vec::new(),
            timeout: Duration::from_secs(600),
            ok_codes: vec![0],
        }
    }

    pub fn arg(mut self, a: impl Into<OsString>) -> Self {
        self.args.push(a.into());
        self
    }

    pub fn args<I, A>(mut self, it: I) -> Self
    where
        I: IntoIterator<Item = A>,
        A: Into<OsString>,
    {
        self.args.extend(it.into_iter().map(Into::into));
        self
    }

    pub fn timeout(mut self, t: Duration) -> Self {
        self.timeout = t;
        self
    }

    /// Additional exit codes treated as success (e.g. rsync 24).
    pub fn ok_code(mut self, c: i32) -> Self {
        self.ok_codes.push(c);
        self
    }

    /// Run to completion. `on_line` sees stdout split on `\n` and `\r`.
    pub async fn run(
        self,
        cancel: &CancellationToken,
        mut on_line: Option<&mut (dyn FnMut(&str) + Send)>,
    ) -> Result<ToolOutput> {
        let mut cmd = tokio::process::Command::new(&self.program);
        cmd.args(&self.args)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .process_group(0)
            .kill_on_drop(true);
        let mut child = cmd.spawn().map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                AppError::external(format!("{} is not installed", self.name))
            } else {
                AppError::external(format!("could not start {}", self.name))
                    .with_detail(e.to_string())
            }
        })?;
        let pgid = child.id().and_then(|p| i32::try_from(p).ok());
        let stderr = child.stderr.take();
        let err_task = tokio::spawn(read_tail(stderr));
        let stdout = child.stdout.take();

        enum End {
            Done(String, std::io::Result<ExitStatus>),
            Timeout,
            Cancelled,
        }
        let end = {
            let work = async {
                let out = read_lines(stdout, &mut on_line).await;
                let st = child.wait().await;
                (out, st)
            };
            tokio::select! {
                (out, st) = work => End::Done(out, st),
                () = tokio::time::sleep(self.timeout) => End::Timeout,
                () = cancel.cancelled() => End::Cancelled,
            }
        };
        let (stdout, status) = match end {
            End::Done(out, st) => (out, st),
            End::Timeout => {
                terminate(&mut child, pgid).await;
                return Err(AppError::timeout(format!(
                    "{} timed out after {} s",
                    self.name,
                    self.timeout.as_secs()
                )));
            }
            End::Cancelled => {
                terminate(&mut child, pgid).await;
                return Err(AppError::cancelled());
            }
        };
        let stderr_tail = err_task.await.unwrap_or_default();
        let status = status.map_err(|e| {
            AppError::external(format!("{} did not finish", self.name)).with_detail(e.to_string())
        })?;
        let code = status.code();
        let out = ToolOutput {
            code,
            stdout,
            stderr_tail,
        };
        if code.is_some_and(|c| self.ok_codes.contains(&c)) {
            return Ok(out);
        }
        let how = match code {
            Some(c) => format!("exit code {c}"),
            None => "killed by a signal".to_string(),
        };
        Err(AppError::external(format!("{} failed ({how})", self.name))
            .with_detail(out.stderr_tail.trim().to_string()))
    }
}

/// SIGTERM the whole group, wait up to `KILL_GRACE`, then SIGKILL.
async fn terminate(child: &mut Child, pgid: Option<i32>) {
    if let Some(pg) = pgid {
        let _ = killpg(Pid::from_raw(pg), Signal::SIGTERM);
        if tokio::time::timeout(KILL_GRACE, child.wait()).await.is_ok() {
            // Leader exited; make sure nothing else in the group lingers.
            let _ = killpg(Pid::from_raw(pg), Signal::SIGKILL);
            return;
        }
        let _ = killpg(Pid::from_raw(pg), Signal::SIGKILL);
    }
    let _ = child.start_kill();
    let _ = child.wait().await;
}

async fn read_lines(
    stdout: Option<impl AsyncRead + Unpin>,
    on_line: &mut Option<&mut (dyn FnMut(&str) + Send)>,
) -> String {
    let Some(mut r) = stdout else {
        return String::new();
    };
    let mut all = Vec::new();
    let mut pending = Vec::new();
    let mut buf = [0u8; 8192];
    loop {
        let n = match r.read(&mut buf).await {
            Ok(0) | Err(_) => break,
            Ok(n) => n,
        };
        if all.len() < STDOUT_CAP {
            all.extend_from_slice(&buf[..n.min(STDOUT_CAP - all.len())]);
        }
        for &b in &buf[..n] {
            if b == b'\n' || b == b'\r' {
                flush_line(&mut pending, on_line);
            } else {
                pending.push(b);
            }
        }
    }
    flush_line(&mut pending, on_line);
    String::from_utf8_lossy(&all).into_owned()
}

fn flush_line(pending: &mut Vec<u8>, on_line: &mut Option<&mut (dyn FnMut(&str) + Send)>) {
    if pending.is_empty() {
        return;
    }
    if let Some(f) = on_line.as_mut() {
        f(&String::from_utf8_lossy(pending));
    }
    pending.clear();
}

async fn read_tail(stderr: Option<impl AsyncRead + Unpin>) -> String {
    let Some(mut r) = stderr else {
        return String::new();
    };
    let mut tail: Vec<u8> = Vec::new();
    let mut buf = [0u8; 4096];
    loop {
        match r.read(&mut buf).await {
            Ok(0) | Err(_) => break,
            Ok(n) => {
                tail.extend_from_slice(&buf[..n]);
                if tail.len() > STDERR_TAIL * 2 {
                    tail.drain(..tail.len() - STDERR_TAIL);
                }
            }
        }
    }
    if tail.len() > STDERR_TAIL {
        tail.drain(..tail.len() - STDERR_TAIL);
    }
    String::from_utf8_lossy(&tail).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Instant;
    use wa_core::ErrorKind;

    fn sh(script: &str) -> Tool {
        Tool::new("sh", Path::new("sh")).arg("-c").arg(script)
    }

    #[tokio::test]
    async fn captures_output_and_lines() {
        let mut lines = Vec::new();
        let mut cb = |l: &str| lines.push(l.to_string());
        let out = sh("printf 'a\\rb\\nc'; echo err >&2")
            .run(&CancellationToken::new(), Some(&mut cb))
            .await
            .unwrap();
        assert_eq!(lines, ["a", "b", "c"]);
        assert_eq!(out.stdout, "a\rb\nc");
        assert_eq!(out.stderr_tail.trim(), "err");
        assert_eq!(out.code, Some(0));
    }

    #[tokio::test]
    async fn missing_tool_message() {
        let e = Tool::new("ffmpeg", Path::new("/nonexistent/ffmpeg-x"))
            .run(&CancellationToken::new(), None)
            .await
            .unwrap_err();
        assert_eq!(e.kind, ErrorKind::External);
        assert_eq!(e.message, "ffmpeg is not installed");
    }

    #[tokio::test]
    async fn nonzero_exit_is_external_with_stderr() {
        let e = sh("echo boom >&2; exit 3")
            .run(&CancellationToken::new(), None)
            .await
            .unwrap_err();
        assert_eq!(e.kind, ErrorKind::External);
        assert!(e.message.contains("exit code 3"));
        assert_eq!(e.detail.as_deref(), Some("boom"));
        assert!(
            sh("exit 24")
                .ok_code(24)
                .run(&CancellationToken::new(), None)
                .await
                .is_ok()
        );
    }

    #[tokio::test]
    async fn timeout_kills() {
        let t0 = Instant::now();
        let e = Tool::new("sleep", Path::new("sleep"))
            .arg("30")
            .timeout(Duration::from_millis(300))
            .run(&CancellationToken::new(), None)
            .await
            .unwrap_err();
        assert_eq!(e.kind, ErrorKind::Timeout);
        assert!(t0.elapsed() < Duration::from_secs(3));
    }

    #[tokio::test]
    async fn cancel_kills_sleep_quickly() {
        let c = CancellationToken::new();
        let c2 = c.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(200)).await;
            c2.cancel();
        });
        let t0 = Instant::now();
        let e = Tool::new("sleep", Path::new("sleep"))
            .arg("30")
            .run(&c, None)
            .await
            .unwrap_err();
        assert_eq!(e.kind, ErrorKind::Cancelled);
        assert!(t0.elapsed() < Duration::from_millis(3500));
    }

    #[tokio::test]
    async fn sigterm_ignorer_gets_sigkill_and_group_dies() {
        let dir = tempfile::tempdir().unwrap();
        let pidfile = dir.path().join("grandchild.pid");
        let c = CancellationToken::new();
        let c2 = c.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(300)).await;
            c2.cancel();
        });
        let t0 = Instant::now();
        let script = format!(
            "trap '' TERM; sh -c 'trap \"\" TERM; sleep 30' & echo $! > {}; wait",
            pidfile.display()
        );
        let e = sh(&script).run(&c, None).await.unwrap_err();
        let el = t0.elapsed();
        assert_eq!(e.kind, ErrorKind::Cancelled);
        assert!(
            el >= Duration::from_millis(3000) && el < Duration::from_millis(4800),
            "{el:?}"
        );
        let pid: i32 = std::fs::read_to_string(&pidfile)
            .unwrap()
            .trim()
            .parse()
            .unwrap();
        tokio::time::sleep(Duration::from_millis(100)).await;
        // The grandchild was in our group and must be gone (or a zombie reaped by init).
        let alive = std::fs::read_to_string(format!("/proc/{pid}/stat"))
            .map(|s| !s.contains(") Z"))
            .unwrap_or(false);
        assert!(!alive, "grandchild survived");
    }
}
