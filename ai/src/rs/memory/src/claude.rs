use std::{
    collections::{HashSet, hash_map::DefaultHasher},
    ffi::{OsStr, OsString},
    fs,
    hash::{Hash, Hasher},
    io::{self, BufRead, BufReader, Read, Write},
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    sync::{
        Arc, Mutex,
        mpsc::{RecvTimeoutError, Sender, channel},
    },
    thread,
    time::{Duration, Instant},
};

use anyhow::{Context, Result, bail};
use serde_json::{Value, json};
use tempfile::TempDir;

use crate::{
    backend::{Backend, Block, Conversation},
    store::AtPath,
};

const CALL_TIMEOUT: Duration = Duration::from_secs(300);
const KILL_GRACE: Duration = Duration::from_millis(200);

/// The `claude` on `PATH`.
pub fn find() -> Result<OsString> {
    let path = std::env::var_os("PATH").unwrap_or_default();
    let found = std::env::split_paths(&path)
        .map(|d| d.join("claude"))
        .find(|p| p.is_file())
        .context("No claude on PATH: MyAI runs on Claude Code.")?;
    Ok(found.into())
}

/// `claude -p` with the flags every call shares: stream-json both ways, isolated from the
/// user's settings, 5-minute cache marks. Callers add their own flags after `--tools`.
pub fn command(program: &OsStr, model: &str, effort: &str, system: &Path, tools: &str) -> Command {
    let mut c = Command::new(program);
    c.args(["-p", "--model", model, "--effort", effort])
        .args([
            "--input-format",
            "stream-json",
            "--output-format",
            "stream-json",
        ])
        .args(["--verbose", "--include-partial-messages"])
        .args([
            "--no-session-persistence",
            "--setting-sources",
            "",
            "--strict-mcp-config",
        ])
        .arg("--system-prompt-file")
        .arg(system)
        .args(["--tools", tools])
        .env("CLAUDE_CODE_PROMPT_CACHE_TTL", "5m")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    die_with_parent(&mut c);
    c
}

/// The child dies even if it ignores SIGTERM when its spawning thread or process ends.
#[cfg(target_os = "linux")]
fn die_with_parent(c: &mut Command) {
    use std::os::unix::process::CommandExt;
    let parent = std::process::id() as libc::pid_t;
    unsafe {
        c.pre_exec(move || {
            if libc::setpgid(0, 0) != 0 {
                return Err(io::Error::last_os_error());
            }
            if libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGKILL) != 0 {
                return Err(io::Error::last_os_error());
            }
            if libc::getppid() != parent {
                return Err(io::Error::other("the parent is gone"));
            }
            Ok(())
        });
    }
}

#[cfg(not(target_os = "linux"))]
fn die_with_parent(_: &mut Command) {}

fn terminate(pid: u32) {
    unsafe {
        libc::kill(-(pid as libc::pid_t), libc::SIGTERM);
        libc::kill(pid as libc::pid_t, libc::SIGTERM);
    }
}

fn kill_group(pid: u32) {
    unsafe {
        libc::kill(-(pid as libc::pid_t), libc::SIGKILL);
        libc::kill(pid as libc::pid_t, libc::SIGKILL);
    }
}

/// Pids of running children, so an owner can end them all.
#[derive(Clone, Default)]
pub struct Live(Arc<Mutex<LiveState>>);

#[derive(Default)]
struct LiveState {
    pids: HashSet<u32>,
    stopped: bool,
}

impl Live {
    /// Ends every registered child and prevents future spawns through this registry.
    pub fn kill_all(&self) {
        let mut live = self.0.lock().expect("live lock");
        live.stopped = true;
        for &pid in &live.pids {
            terminate(pid);
            kill_group(pid);
        }
    }
}

/// A running `claude`: stream-json messages in, stdout lines out through a sink.
pub struct Proc {
    child: Option<Child>,
    stdin: Option<Sender<String>>,
    stderr: Arc<Mutex<String>>,
    live: Option<Live>,
}

impl Proc {
    /// Starts `cmd`; a reader thread hands `sink` each stdout line, then `None` at its end.
    pub fn spawn(
        cmd: Command,
        live: Option<&Live>,
        sink: impl FnMut(Option<String>) + Send + 'static,
    ) -> Result<Proc> {
        Self::spawn_until(cmd, live, None, sink)
    }

    /// Ends a one-shot call at its terminal event before handing it to a slower consumer.
    pub fn spawn_until(
        mut cmd: Command,
        live: Option<&Live>,
        stop: Option<fn(&Value) -> bool>,
        mut sink: impl FnMut(Option<String>) + Send + 'static,
    ) -> Result<Proc> {
        let mut registry = live.map(|l| l.0.lock().expect("live lock"));
        if registry.as_ref().is_some_and(|l| l.stopped) {
            bail!("claude owner has stopped");
        }
        let child = cmd
            .spawn()
            .context("Cannot run claude. Is Claude Code installed and on PATH?")?;
        if let Some(l) = &mut registry {
            l.pids.insert(child.id());
        }
        drop(registry);
        let mut proc = Proc {
            child: Some(child),
            stdin: None,
            stderr: Arc::new(Mutex::new(String::new())),
            live: live.cloned(),
        };
        let child = proc.child.as_mut().expect("spawned child");
        let stdout = child.stdout.take().context("No stdout.")?;
        let mut err = child.stderr.take().context("No stderr.")?;
        let pid = child.id();
        proc.stdin = child.stdin.take().map(|mut pipe| {
            let (tx, rx) = channel::<String>();
            thread::spawn(move || {
                for message in rx {
                    if writeln!(pipe, "{message}")
                        .and_then(|()| pipe.flush())
                        .is_err()
                    {
                        break;
                    }
                }
            });
            tx
        });
        thread::spawn(move || {
            for line in BufReader::new(stdout).lines().map_while(Result::ok) {
                let finished =
                    stop.is_some_and(|stop| serde_json::from_str(&line).is_ok_and(|ev| stop(&ev)));
                if finished {
                    terminate(pid);
                    kill_group(pid);
                }
                sink(Some(line));
                if finished {
                    break;
                }
            }
            sink(None);
        });
        let buf = proc.stderr.clone();
        thread::spawn(move || {
            let mut chunk = [0; 4096];
            while let Ok(n @ 1..) = err.read(&mut chunk) {
                buf.lock()
                    .expect("stderr lock")
                    .push_str(&String::from_utf8_lossy(&chunk[..n]));
            }
        });
        Ok(proc)
    }

    pub fn pid(&self) -> u32 {
        self.child.as_ref().map_or(0, Child::id)
    }

    /// Queues one stream-json user message without blocking on the child reading stdin.
    pub fn send(&mut self, blocks: &[Block]) -> io::Result<()> {
        let stdin = self.stdin.as_ref().ok_or(io::ErrorKind::BrokenPipe)?;
        stdin
            .send(user_message(blocks))
            .map_err(|_| io::ErrorKind::BrokenPipe.into())
    }

    /// Sends SIGTERM to the child group; dropping it bounds the wait and reaps it.
    pub fn kill(&self) {
        if let Some(c) = &self.child {
            terminate(c.id());
        }
    }

    /// Why the child ended without a result: its exit code and the tail of its stderr.
    pub fn failure(&mut self) -> String {
        if let Some(child) = &self.child {
            kill_group(child.id());
        }
        let code = self.child.as_mut().and_then(|c| c.wait().ok()?.code());
        thread::sleep(Duration::from_millis(50));
        let err = self.stderr.lock().expect("stderr lock").clone();
        let err = match err.trim() {
            "" => "no error output",
            e => tail(e, 300),
        };
        format!("claude exited (code {code:?}) before its result: {err}")
    }
}

impl Drop for Proc {
    fn drop(&mut self) {
        let Some(mut child) = self.child.take() else {
            return;
        };
        self.stdin.take();
        terminate(child.id());
        let deadline = Instant::now() + KILL_GRACE;
        while Instant::now() < deadline && !matches!(child.try_wait(), Ok(Some(_))) {
            thread::sleep(Duration::from_millis(10));
        }
        kill_group(child.id());
        if !matches!(child.try_wait(), Ok(Some(_))) {
            let _ = child.kill();
            let _ = child.wait();
        }
        if let Some(l) = &self.live {
            l.0.lock().expect("live lock").pids.remove(&child.id());
        }
    }
}

/// Writes each distinct system prompt once into a temporary directory.
pub struct Prompts(TempDir);

impl Prompts {
    pub fn new() -> Result<Prompts> {
        Ok(Prompts(
            TempDir::new().context("Cannot create a temporary directory.")?,
        ))
    }

    pub fn file(&self, system: &str) -> Result<PathBuf> {
        let mut h = DefaultHasher::new();
        system.hash(&mut h);
        let p = self.0.path().join(format!("{:016x}.txt", h.finish()));
        if !p.exists() {
            fs::write(&p, system).at(&p)?;
        }
        Ok(p)
    }
}

/// Claude Code in print mode (`claude -p`), isolated from the user's settings, without tools.
pub struct ClaudeCode {
    program: OsString,
    model: String,
    effort: String,
    prompts: Prompts,
    live: Live,
}

impl ClaudeCode {
    /// The compactor's model: Sonnet at medium effort, through the `claude` on `PATH`.
    pub fn compactor() -> Result<ClaudeCode> {
        ClaudeCode::compactor_at(find()?)
    }

    pub fn compactor_at(program: OsString) -> Result<ClaudeCode> {
        Ok(ClaudeCode {
            program,
            model: "sonnet".into(),
            effort: "medium".into(),
            prompts: Prompts::new()?,
            live: Live::default(),
        })
    }

    pub fn command(&self, system_file: &Path) -> Command {
        let mut c = command(&self.program, &self.model, &self.effort, system_file, "");
        c.arg("--safe-mode").env("DISABLE_PROMPT_CACHING", "1");
        c
    }
}

impl Backend for ClaudeCode {
    fn agent(&self) -> &str {
        "claude-code"
    }

    fn model(&self) -> &str {
        &self.model
    }

    fn open(&self, system: &str) -> Result<Box<dyn Conversation>> {
        let (tx, lines) = channel();
        let proc = Proc::spawn(
            self.command(&self.prompts.file(system)?),
            Some(&self.live),
            move |l| {
                if let Some(l) = l {
                    let _ = tx.send(l);
                }
            },
        )?;
        Ok(Box::new(Session {
            proc,
            lines,
            id: String::new(),
        }))
    }

    fn stop(&self) {
        self.live.kill_all();
    }
}

struct Session {
    proc: Proc,
    lines: std::sync::mpsc::Receiver<String>,
    id: String,
}

/// A stream-json user message.
pub fn user_message(message: &[Block]) -> String {
    let content: Vec<Value> = message
        .iter()
        .map(|b| match b.cache {
            true => json!({"type": "text", "text": b.text, "cache_control": {"type": "ephemeral"}}),
            false => json!({"type": "text", "text": b.text}),
        })
        .collect();
    json!({"type": "user", "message": {"role": "user", "content": content}}).to_string()
}

/// The reply a `result` event carries.
pub fn reply(ev: &Value) -> Result<String> {
    let text = ev["result"].as_str().unwrap_or_default().to_string();
    if ev["is_error"].as_bool().unwrap_or(false) {
        let why = if text.is_empty() {
            ev["subtype"].as_str().unwrap_or("error").to_string()
        } else {
            text
        };
        bail!("claude: {}", tail(&why, 300))
    } else if ev["stop_reason"] == "refusal" {
        bail!("claude refused")
    }
    Ok(text)
}

pub fn tail(s: &str, n: usize) -> &str {
    let s = s.trim();
    &s[s.ceil_char_boundary(s.len().saturating_sub(n))..]
}

impl Conversation for Session {
    fn say(&mut self, message: &[Block]) -> Result<String> {
        let _ = self.proc.send(message);
        let deadline = Instant::now() + CALL_TIMEOUT;
        loop {
            let wait = deadline.saturating_duration_since(Instant::now());
            match self.lines.recv_timeout(wait) {
                Ok(line) => {
                    let Ok(ev) = serde_json::from_str::<Value>(&line) else {
                        continue;
                    };
                    if let Some(id) = ev["session_id"].as_str() {
                        self.id = id.into();
                    }
                    if ev["type"] == "result" {
                        return reply(&ev);
                    }
                }
                Err(RecvTimeoutError::Timeout) => {
                    bail!("claude: no result after {} s.", CALL_TIMEOUT.as_secs())
                }
                Err(RecvTimeoutError::Disconnected) => bail!("{}", self.proc.failure()),
            }
        }
    }

    fn session(&self) -> String {
        self.id.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_compactor_runs_isolated_without_tools() {
        let c = ClaudeCode::compactor_at("claude".into()).unwrap();
        let file = c.prompts.file("You write the memory.").unwrap();
        assert_eq!(fs::read_to_string(&file).unwrap(), "You write the memory.");
        assert_eq!(c.prompts.file("You write the memory.").unwrap(), file);
        let cmd = c.command(&file);
        assert_eq!(cmd.get_program(), "claude");
        let args: Vec<_> = cmd.get_args().map(|a| a.to_str().unwrap()).collect();
        let f = file.to_str().unwrap();
        assert_eq!(
            args,
            [
                "-p",
                "--model",
                "sonnet",
                "--effort",
                "medium",
                "--input-format",
                "stream-json",
                "--output-format",
                "stream-json",
                "--verbose",
                "--include-partial-messages",
                "--no-session-persistence",
                "--setting-sources",
                "",
                "--strict-mcp-config",
                "--system-prompt-file",
                f,
                "--tools",
                "",
                "--safe-mode",
            ]
        );
        let mut env: Vec<_> = cmd
            .get_envs()
            .map(|(k, v)| (k.to_str().unwrap(), v.and_then(|v| v.to_str())))
            .collect();
        env.sort();
        assert_eq!(
            env,
            [
                ("CLAUDE_CODE_PROMPT_CACHE_TTL", Some("5m")),
                ("DISABLE_PROMPT_CACHING", Some("1"))
            ]
        );
        assert_eq!(c.model(), "sonnet");
    }

    #[test]
    fn stream_json_in_and_out() {
        let m = user_message(&[Block::new("a", true), Block::new("b", false)]);
        let v: Value = serde_json::from_str(&m).unwrap();
        assert_eq!(v["type"], "user");
        assert_eq!(
            v["message"]["content"][0]["cache_control"]["type"],
            "ephemeral"
        );
        assert!(v["message"]["content"][1].get("cache_control").is_none());
        let result = |line: &str| reply(&serde_json::from_str(line).unwrap());
        let ok = result(
            r#"{"type":"result","is_error":false,"result":" a line ","stop_reason":"end_turn"}"#,
        );
        assert_eq!(ok.unwrap(), " a line ");
        let err = result(r#"{"type":"result","is_error":true,"subtype":"error_max_turns"}"#);
        assert!(err.unwrap_err().to_string().contains("error_max_turns"));
        let refused =
            result(r#"{"type":"result","is_error":false,"result":"","stop_reason":"refusal"}"#);
        assert!(refused.is_err());
    }

    #[test]
    fn children_end_with_their_owner() {
        let live = Live::default();
        let mut c = Command::new("sleep");
        c.arg("60").stdin(Stdio::piped()).stdout(Stdio::piped());
        c.stderr(Stdio::piped());
        die_with_parent(&mut c);
        let (tx, rx) = channel();
        let p = Proc::spawn(c, Some(&live), move |l| {
            let _ = tx.send(l);
        })
        .unwrap();
        live.kill_all();
        assert_eq!(rx.recv_timeout(Duration::from_secs(5)).unwrap(), None);
        drop(p);
        assert!(live.0.lock().unwrap().pids.is_empty());
    }
}

#[cfg(all(test, target_os = "linux"))]
mod cleanup_tests {
    use super::*;
    fn shell(code: &str) -> Command {
        let mut c = Command::new("bash");
        c.args(["-c", code])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        die_with_parent(&mut c);
        c
    }
    fn running(pid: u32) -> bool {
        fs::read_to_string(format!("/proc/{pid}/stat"))
            .is_ok_and(|s| s.split_whitespace().nth(2).is_none_or(|state| state != "Z"))
    }
    #[test]
    fn eof_from_a_live_ignoring_term_child_is_bounded_and_reaped() {
        let (tx, rx) = channel();
        let mut proc = Proc::spawn(shell("trap '' TERM; exec 1>&-; sleep 60"), None, move |l| {
            let _ = tx.send(l);
        })
        .unwrap();
        assert_eq!(rx.recv_timeout(Duration::from_secs(2)).unwrap(), None);
        let pid = proc.pid();
        let at = Instant::now();
        let why = proc.failure();
        drop(proc);
        assert!(why.contains("before its result"));
        assert!(at.elapsed() < Duration::from_secs(1));
        assert!(!running(pid));
    }
    #[test]
    fn dropping_owner_ends_ignoring_term_descendants_and_prevents_new_calls() {
        let live = Live::default();
        let (tx, rx) = channel();
        let proc = Proc::spawn(
            shell("trap '' TERM; sleep 60 & echo $!; wait"),
            Some(&live),
            move |l| {
                let _ = tx.send(l);
            },
        )
        .unwrap();
        let child: u32 = rx
            .recv_timeout(Duration::from_secs(2))
            .unwrap()
            .unwrap()
            .parse()
            .unwrap();
        assert!(running(child));
        let at = Instant::now();
        drop(proc);
        assert!(at.elapsed() < Duration::from_secs(1));
        let deadline = Instant::now() + Duration::from_secs(1);
        while running(child) && Instant::now() < deadline {
            thread::sleep(Duration::from_millis(10));
        }
        assert!(!running(child));
        live.kill_all();
        assert!(Proc::spawn(shell("sleep 60"), Some(&live), |_| {}).is_err());
    }
    #[test]
    fn terminal_event_kills_before_a_slow_consumer_and_followup() {
        let temp = TempDir::new().unwrap();
        let sentinel = temp.path().join("followup");
        let script = format!(
            "for i in {{1..50}}; do echo '{{\"type\":\"assistant\"}}'; done; echo '{{\"type\":\"result\"}}'; sleep .02; touch '{}'; sleep 60",
            sentinel.display()
        );
        let (tx, rx) = channel();
        let proc = Proc::spawn_until(
            shell(&script),
            None,
            Some(|ev| ev["type"] == "result"),
            move |line| {
                let _ = tx.send(line);
            },
        )
        .unwrap();
        thread::sleep(Duration::from_millis(100));
        assert!(!sentinel.exists());
        let lines: Vec<_> = rx.try_iter().flatten().collect();
        assert_eq!(lines.last().unwrap(), "{\"type\":\"result\"}");
        drop(proc);
    }
    #[test]
    fn parent_death_kills_an_ignoring_term_claude() {
        let mut owner = Command::new("bash");
        // A separate copy of this test executable owns the PDEATHSIG-protected child.
        let temp = TempDir::new().unwrap();
        let pidfile = temp.path().join("pid");
        owner.arg("-c").arg("exec \"$1\" --ignored --exact claude::cleanup_tests::parent_death_helper --nocapture").arg("owner").arg(std::env::current_exe().unwrap()).env("MYAI_CHILD_PIDFILE", &pidfile).stdout(Stdio::null()).stderr(Stdio::null());
        let mut owner = owner.spawn().unwrap();
        let deadline = Instant::now() + Duration::from_secs(3);
        while !pidfile.exists() {
            assert!(Instant::now() < deadline);
            thread::sleep(Duration::from_millis(10));
        }
        let pid: u32 = fs::read_to_string(&pidfile).unwrap().parse().unwrap();
        owner.kill().unwrap();
        owner.wait().unwrap();
        let deadline = Instant::now() + Duration::from_secs(2);
        while running(pid) && Instant::now() < deadline {
            thread::sleep(Duration::from_millis(10));
        }
        assert!(!running(pid));
    }
    #[test]
    #[ignore]
    fn parent_death_helper() {
        let p = Proc::spawn(shell("trap '' TERM; while :; do :; done"), None, |_| {}).unwrap();
        fs::write(
            std::env::var_os("MYAI_CHILD_PIDFILE").unwrap(),
            p.pid().to_string(),
        )
        .unwrap();
        thread::sleep(Duration::from_secs(60));
    }
}
