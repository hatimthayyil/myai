use std::{
    io::Read,
    process::{Command, Stdio},
    thread,
    time::{Duration, Instant},
};

use anyhow::Result;

use crate::{
    merge::{first_change, merge},
    nap::pending_count,
    store::{REF, Store},
};

const TRIES: usize = 3;
const NO_REF: &str = "couldn't find remote ref";

/// The remote this memory syncs with: `ai.memory.remote`, else `origin` if it exists.
pub fn remote(s: &Store) -> Option<String> {
    s.setting("ai.memory.remote")
        .or_else(|| s.setting("remote.origin.url").map(|_| "origin".into()))
}

struct Git<'a> {
    store: &'a Store,
    deadline: Option<Instant>,
}

impl Git<'_> {
    /// Runs git on the memory; `Err` holds git's reason.
    fn run(&self, args: &[&str]) -> std::result::Result<(), String> {
        let mut c = Command::new("git");
        c.arg("--git-dir")
            .arg(self.store.dir())
            .args(args)
            .env("GIT_TERMINAL_PROMPT", "0")
            .env("LC_ALL", "C")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::piped());
        let custom = ["GIT_SSH", "GIT_SSH_COMMAND"]
            .iter()
            .any(|k| std::env::var_os(k).is_some())
            || self.store.setting("core.sshCommand").is_some();
        if !custom {
            c.env(
                "GIT_SSH_COMMAND",
                "ssh -o ConnectTimeout=3 -o BatchMode=yes",
            );
        }
        let mut child = c.spawn().map_err(|e| format!("cannot run git: {e}"))?;
        let mut pipe = child.stderr.take().expect("piped");
        let reader = thread::spawn(move || {
            let mut s = String::new();
            let _ = pipe.read_to_string(&mut s);
            s
        });
        let status = loop {
            match child.try_wait().map_err(|e| e.to_string())? {
                Some(st) => break st,
                None if self.deadline.is_some_and(|d| Instant::now() >= d) => {
                    let _ = child.kill();
                    let _ = child.wait();
                    return Err("no answer in time".into());
                }
                None => thread::sleep(Duration::from_millis(10)),
            }
        };
        let err = reader.join().unwrap_or_default();
        if status.success() {
            return Ok(());
        }
        let lines: Vec<_> = err
            .lines()
            .map(str::trim)
            .filter(|l| !l.is_empty())
            .collect();
        let why = lines
            .iter()
            .find(|l| l.starts_with("fatal:") || l.starts_with("error:"))
            .or(lines.last())
            .map_or("git failed", |l| l);
        Err(why
            .trim_start_matches("fatal:")
            .trim_start_matches("error:")
            .trim()
            .trim_end_matches('.')
            .to_string())
    }
}

pub struct Report {
    pub warning: Option<String>,
    pub clashes: u64,
    pub pushed: bool,
    pub taken: Option<Taken>,
}

/// What a sync changed in the local memory.
pub struct Taken {
    pub memories: u64,
    pub renumbered: Option<u64>,
    pub redo: u64,
}

/// Fetches from `remote`, merges, and pushes back, retrying when the push loses a race.
pub fn sync(s: &Store, remote: &str, deadline: Option<Instant>) -> Result<Report> {
    let git = Git { store: s, deadline };
    let tracking = format!("refs/ai/remotes/{remote}/memory");
    let fetch = format!("+{REF}:{tracking}");
    let push = format!("{REF}:{REF}");
    let before = s.snapshot()?;
    let mut r = Report {
        warning: None,
        clashes: 0,
        pushed: false,
        taken: None,
    };
    for _ in 0..TRIES {
        let fetched = git.run(&["fetch", "--quiet", "--no-write-fetch-head", remote, &fetch]);
        let theirs = s.resolve(&tracking)?;
        if let Some(c) = theirs {
            r.clashes += merge(s, c, &format!("merge {remote}"))?;
        }
        if let Some(e) = fetched.err().filter(|e| !e.contains(NO_REF)) {
            r.warning = Some(format!("cannot reach {remote}: {e}"));
            break;
        }
        let ours = s.head()?;
        if ours.is_none() || ours == theirs {
            r.warning = None;
            break;
        }
        match git.run(&["push", "--quiet", remote, &push]) {
            Ok(()) => {
                (r.pushed, r.warning) = (true, None);
                break;
            }
            Err(e) => r.warning = Some(format!("cannot push to {remote}: {e}")),
        }
    }
    let _ = git.run(&["gc", "--auto", "--quiet"]);
    let after = s.snapshot()?;
    if after.tree() != before.tree() {
        let t = after.log_len()?;
        r.taken = Some(Taken {
            memories: t - before.log_len()?,
            renumbered: first_change(&before, &after)?,
            redo: pending_count(&after, t)?,
        });
    }
    Ok(r)
}
