use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
    process::{Command, Output, Stdio},
    thread,
    time::{Duration, Instant, SystemTime},
};

use ai::memory::{Coord, Message, REF, Store};
use tempfile::TempDir;

const ISOLATE: [&str; 17] = [
    "AI_MEMORY_DIR",
    "AI_AGENT",
    "AI_MODEL",
    "AI_SESSION",
    "CLAUDECODE",
    "CLAUDE_CODE_SESSION_ID",
    "CODEX_THREAD_ID",
    "CODEX_SANDBOX",
    "CODEX_SANDBOX_NETWORK_DISABLED",
    "GIT_DIR",
    "GIT_AUTHOR_NAME",
    "GIT_AUTHOR_EMAIL",
    "GIT_COMMITTER_NAME",
    "GIT_COMMITTER_EMAIL",
    "EMAIL",
    "GIT_SSH",
    "GIT_SSH_COMMAND",
];

struct Sandbox {
    tmp: TempDir,
}

impl Sandbox {
    fn new() -> Sandbox {
        let tmp = TempDir::new().unwrap();
        assert!(
            !tmp.path().ancestors().any(|a| a.join(".git").exists()),
            "the temp dir is inside a git repository"
        );
        fs::create_dir(tmp.path().join("home")).unwrap();
        Sandbox { tmp }
    }

    fn path(&self) -> &Path {
        self.tmp.path()
    }

    fn store(&self) -> PathBuf {
        self.path().join("m")
    }

    fn isolate(&self, c: &mut Command) {
        let home = self.path().join("home");
        c.env("HOME", &home)
            .env("XDG_CONFIG_HOME", home.join(".config"))
            .env("GIT_CONFIG_NOSYSTEM", "1");
        for k in ISOLATE {
            c.env_remove(k);
        }
        c.env("AI_MEMORY_NAP", "0");
    }

    fn cmd(&self, cwd: &Path, store: Option<&Path>) -> Command {
        let mut c = Command::new(env!("CARGO_BIN_EXE_ai"));
        self.isolate(&mut c);
        c.current_dir(cwd).arg("memory");
        if let Some(d) = store {
            c.env("AI_MEMORY_DIR", d);
        }
        c
    }

    fn ai(&self, args: &[&str]) -> Output {
        self.ai_at(&self.store(), args)
    }

    fn ai_at(&self, d: &Path, args: &[&str]) -> Output {
        self.cmd(self.path(), Some(d)).args(args).output().unwrap()
    }

    fn git(&self, dir: &Path, args: &[&str]) -> String {
        let mut c = Command::new("git");
        self.isolate(&mut c);
        let r = c
            .current_dir(dir)
            .args(["-c", "user.name=T", "-c", "user.email=t@t"])
            .args(["-c", "maintenance.auto=false", "-c", "gc.auto=0"])
            .args(args)
            .output()
            .unwrap();
        assert!(r.status.success(), "git {args:?}: {}", text(&r.stderr));
        text(&r.stdout).trim_end().to_string()
    }

    fn store_git(&self, args: &[&str]) -> String {
        self.git_at(&self.store(), args)
    }

    fn git_at(&self, d: &Path, args: &[&str]) -> String {
        let dir = format!("--git-dir={}", d.display());
        self.git(self.path(), &[&[dir.as_str()], args].concat())
    }

    fn memories(&self) -> Vec<Message> {
        let st = Store::open(&self.store()).unwrap();
        let snap = st.snapshot().unwrap();
        let mut out = Vec::new();
        snap.scan(u64::MAX, |_, m| {
            out.push(m);
            Ok(())
        })
        .unwrap();
        out
    }
}

fn text(b: &[u8]) -> String {
    String::from_utf8_lossy(b).into_owned()
}

#[test]
fn defaults_to_the_home_dir() {
    let s = Sandbox::new();
    let r = s.cmd(s.path(), None).arg("init").output().unwrap();
    assert!(r.status.success(), "{}", text(&r.stderr));
    assert!(text(&r.stdout).starts_with("Created ~/.ai/memory: your memory."));
    let d = s.path().join("home/.ai/memory");
    assert_eq!(
        s.git(
            s.path(),
            &[
                &format!("--git-dir={}", d.display()),
                "rev-parse",
                "--is-bare-repository"
            ]
        ),
        "true"
    );
    let r = s.cmd(s.path(), None).arg("wake").output().unwrap();
    assert!(r.status.success() && text(&r.stdout).contains("You are awake."));
}

#[test]
fn exit_codes_and_streams() {
    let s = Sandbox::new();
    let d = s.store();
    let r = s.ai(&["wake"]);
    assert_eq!(r.status.code(), Some(1));
    assert!(text(&r.stderr).contains("No memory at") && text(&r.stderr).contains("ai memory init"));
    assert!(!d.exists());

    assert!(s.ai(&["init"]).status.success());
    let r = s.ai(&["wake"]);
    assert!(r.status.success() && text(&r.stdout).contains("No memories yet"));
    assert!(!s.ai(&["zoom"]).status.success());

    assert!(s.ai(&["note", "memory 0"]).status.success());
    let r = s.ai(&["zoom", "0+2"]);
    assert_eq!(r.status.code(), Some(1));
    assert_eq!(text(&r.stderr), "No line 0+2.\n");
    assert!(!s.ai(&["frobnicate"]).status.success());
}

#[test]
fn parallel_processes_lose_nothing() {
    let s = Sandbox::new();
    assert!(s.ai(&["init"]).status.success());
    let p = 16;
    let kids: Vec<_> = (0..p)
        .map(|i| {
            s.cmd(s.path(), Some(&s.store()))
                .args(["note", &format!("parallel note {i}")])
                .stdout(Stdio::null())
                .spawn()
                .unwrap()
        })
        .collect();
    for mut k in kids {
        assert!(k.wait().unwrap().success());
    }
    let all = s.memories();
    let mut texts: Vec<_> = all.iter().map(|m| m.text.clone()).collect();
    texts.sort();
    let mut want: Vec<_> = (0..p).map(|i| format!("parallel note {i}")).collect();
    want.sort();
    assert_eq!(texts, want);
    assert!(all.windows(2).all(|w| w[0].ts <= w[1].ts));
    let log = s.store_git(&["log", "--format=%s", REF]);
    assert_eq!(log.lines().filter(|l| *l == "note").count(), p);
}

#[test]
fn provenance_comes_from_the_cwd_and_env() {
    let s = Sandbox::new();
    assert!(s.ai(&["init"]).status.success());
    let work = s.path().join("work");
    let sub = work.join("deep/er");
    fs::create_dir_all(&sub).unwrap();
    s.git(&work, &["init", "-q", "-b", "trunk"]);
    s.git(&work, &["commit", "-q", "--allow-empty", "-m", "c"]);
    s.git(
        &work,
        &["remote", "add", "origin", "git@github.com:acme/widget.git"],
    );
    let head = s.git(&work, &["rev-parse", "--short=12", "HEAD"]);

    let r = s
        .cmd(&sub, Some(&s.store()))
        .args(["note", "noted inside a repo"])
        .env("CLAUDECODE", "1")
        .env("CLAUDE_CODE_SESSION_ID", "sess-1")
        .env("AI_MODEL", "big model")
        .output()
        .unwrap();
    assert!(r.status.success(), "{}", text(&r.stderr));
    let r = s
        .cmd(s.path(), Some(&s.store()))
        .args(["note", "noted outside any repo"])
        .env("CLAUDECODE", "1")
        .env("AI_AGENT", "pi")
        .env("AI_SESSION", "s2")
        .output()
        .unwrap();
    assert!(r.status.success(), "{}", text(&r.stderr));

    let m = s.memories();
    let (a, b) = (&m[0], &m[1]);
    assert_eq!(
        (
            a.place.repo.as_str(),
            a.place.head.as_str(),
            a.place.branch.as_str()
        ),
        ("acme/widget", head.as_str(), "trunk")
    );
    assert_eq!(
        (
            a.who.agent.as_str(),
            a.who.model.as_str(),
            a.who.session.as_str()
        ),
        ("claude-code", "big_model", "sess-1")
    );
    assert_eq!(
        (
            b.place.repo.as_str(),
            b.place.head.as_str(),
            b.place.branch.as_str()
        ),
        ("-", "-", "-")
    );
    assert_eq!(
        (
            b.who.agent.as_str(),
            b.who.model.as_str(),
            b.who.session.as_str()
        ),
        ("pi", "-", "s2")
    );
    assert!(a.ts.len() == 16 && a.ts.ends_with('Z') && a.ts <= b.ts);

    s.git(&work, &["remote", "remove", "origin"]);
    s.cmd(&work, Some(&s.store()))
        .args(["note", "noted in a repo with no remote"])
        .output()
        .unwrap();
    assert_eq!(s.memories()[2].place.repo, "work");

    let show = text(&s.ai(&["show", "0+1"]).stdout);
    for f in [
        "kind    note\n".into(),
        "repo    acme/widget\n".into(),
        format!("head    {head}\n"),
        "branch  trunk\n".into(),
        "agent   claude-code\n".into(),
        "model   big_model\n".into(),
        "session sess-1\n".into(),
        "text    noted inside a repo\n".into(),
    ] {
        assert!(show.contains(&f), "{f:?} missing from:\n{show}");
    }
    let grep = |args: &[&str]| -> String {
        let r = s.ai(&[&["grep", "noted"], args].concat());
        assert!(r.status.success(), "{}", text(&r.stderr));
        text(&r.stdout)
    };
    let first = grep(&[]).lines().next().unwrap().to_string();
    let (date, time) = (&a.ts[..8], &a.ts[9..13]);
    assert_eq!(
        first,
        format!(
            "0+1 {}-{}-{} {}:{} acme/widget note: noted inside a repo",
            &date[..4],
            &date[4..6],
            &date[6..],
            &time[..2],
            &time[2..]
        )
    );
    let heads = |args: &[&str]| -> Vec<String> {
        grep(args)
            .lines()
            .map(|l| l.split(' ').next().unwrap().to_string())
            .collect()
    };
    assert_eq!(heads(&["--repo", "acme/widget"]), ["0+1", "1"]);
    assert_eq!(heads(&["--repo", "work"]), ["2+1", "1"]);
    assert_eq!(heads(&["--agent", "pi"]), ["1+1", "1"]);
    assert_eq!(heads(&["--session", "SESS-1"]), ["0+1", "1"]);
    assert_eq!(heads(&["--kind", "note"]), ["0+1", "1+1", "2+1", "3"]);
    assert_eq!(
        heads(&["--since", "2000-01-01", "--agent", "claude-code"]),
        ["0+1", "1"]
    );
    assert_eq!(grep(&["--agent", "pi", "--repo", "work"]), "No match.\n");
    assert_eq!(grep(&["--since", "2999-01-01"]), "No match.\n");
}

#[test]
fn every_mutation_is_one_named_commit() {
    let s = Sandbox::new();
    s.ai(&["init"]);
    assert_eq!(s.store_git(&["for-each-ref", "--format=%(refname)"]), "");
    let seed = s.path().join("seed.txt");
    fs::write(&seed, "2020-01-01 an imported memory\n").unwrap();
    s.ai(&["import", seed.to_str().unwrap()]);
    s.ai(&["note", "a noted memory"]);
    s.ai(&["wake"]);
    s.ai(&["nap"]);
    assert_eq!(
        s.store_git(&["log", "--format=%s|%cn <%ce>", REF]),
        "note|ai memory <ai@localhost>\n\
         import|ai memory <ai@localhost>"
    );
    assert_eq!(
        s.store_git(&["ls-tree", "-r", "--name-only", REF]),
        "log/0000/00\nlog/0000/01\ntree/0/0000/00\ntree/1/0000/00"
    );
    assert_eq!(s.store_git(&["for-each-ref", "--format=%(refname)"]), REF);
    s.store_git(&["fsck", "--strict", "--no-dangling"]);
}

fn listing(root: &Path, skip: &Path) -> BTreeMap<PathBuf, (u64, SystemTime)> {
    let mut out = BTreeMap::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(p) = stack.pop() {
        for e in fs::read_dir(p).unwrap() {
            let p = e.unwrap().path();
            if p == skip {
                continue;
            }
            let m = fs::symlink_metadata(&p).unwrap();
            if m.is_dir() {
                stack.push(p.clone());
            }
            out.insert(p, (m.len(), m.modified().unwrap()));
        }
    }
    out
}

#[test]
fn nothing_is_written_outside_the_store() {
    let s = Sandbox::new();
    let work = s.path().join("work");
    fs::create_dir(&work).unwrap();
    s.git(&work, &["init", "-q"]);
    s.git(&work, &["commit", "-q", "--allow-empty", "-m", "c"]);
    let seed = s.path().join("seed.txt");
    fs::write(&seed, "2020-01-01 an imported memory\n").unwrap();
    let before = listing(s.path(), &s.store());
    let d = s.store();
    for args in [
        vec!["init"],
        vec!["import", seed.to_str().unwrap()],
        vec!["note", "first"],
        vec!["note", "second"],
        vec!["wake"],
        vec!["config", "PART_LINES=4"],
        vec!["grep", "first", "-t"],
        vec!["zoom", "0+2"],
        vec!["show", "0+2"],
    ] {
        let r = s.cmd(&work, Some(&d)).args(&args).output().unwrap();
        assert!(
            r.status.code().is_some_and(|c| c <= 1) && text(&r.stderr).is_empty(),
            "{args:?}: {}",
            text(&r.stderr)
        );
    }
    assert_eq!(listing(s.path(), &d), before);
    let mut top: Vec<_> = fs::read_dir(&d)
        .unwrap()
        .map(|e| e.unwrap().file_name().into_string().unwrap())
        .collect();
    top.sort();
    assert_eq!(
        top,
        [
            "HEAD",
            "config",
            "description",
            "hooks",
            "info",
            "objects",
            "refs"
        ]
    );
}

#[test]
fn a_closed_pipe_is_quiet() {
    let s = Sandbox::new();
    s.ai(&["init"]);
    let seed = s.path().join("seed.txt");
    let lines: String = (0..5000)
        .map(|i| format!("2020-01-01 memory number {i} padded to make the output long enough\n"))
        .collect();
    fs::write(&seed, lines).unwrap();
    s.ai(&["import", seed.to_str().unwrap()]);
    let mut child = s
        .cmd(s.path(), Some(&s.store()))
        .args(["grep", "memory"])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    drop(child.stdout.take());
    let r = child.wait_with_output().unwrap();
    assert!(r.status.success(), "{:?}", r.status);
    assert_eq!(text(&r.stderr), "");
}

/// A fake `claude`: answers every stream-json message with a fixed result.
const FAKE_CLAUDE: &str = "#!/bin/sh
while read -r line; do
  printf '%s\\n' '{\"type\":\"result\",\"is_error\":false,\"result\":\"a fake summary\",\"stop_reason\":\"end_turn\"}'
done
";

#[test]
fn a_note_naps_in_the_background() {
    let s = Sandbox::new();
    assert!(s.ai(&["init"]).status.success());
    let bin = s.path().join("bin");
    fs::create_dir(&bin).unwrap();
    let claude = bin.join("claude");
    fs::write(&claude, FAKE_CLAUDE).unwrap();
    fs::set_permissions(&claude, std::os::unix::fs::PermissionsExt::from_mode(0o755)).unwrap();
    let path = format!("{}:{}", bin.display(), std::env::var("PATH").unwrap());
    for i in 0..2 {
        let r = s
            .cmd(s.path(), Some(&s.store()))
            .args(["note", &format!("note {i} {}", "z".repeat(400))])
            .env("AI_MEMORY_NAP", "1")
            .env("PATH", &path)
            .output()
            .unwrap();
        assert!(r.status.success(), "{}", text(&r.stderr));
        assert_eq!(text(&r.stdout), format!("Saved as {i}+1.\n"));
    }
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        let st = Store::open(&s.store()).unwrap();
        if let Some(n) = st.snapshot().unwrap().node(Coord::new(1, 0)).unwrap() {
            assert_eq!(
                (n.text.as_str(), n.model.as_str()),
                ("a fake summary", "sonnet")
            );
            break;
        }
        assert!(
            Instant::now() < deadline,
            "no nap: {}",
            fs::read_to_string(s.store().join("nap.log")).unwrap_or_default()
        );
        thread::sleep(Duration::from_millis(50));
    }
    let r = s
        .cmd(s.path(), Some(&s.store()))
        .arg("nap")
        .env("PATH", &path)
        .output()
        .unwrap();
    let out = text(&r.stdout);
    assert!(
        out == "Nothing to build.\n" || out.starts_with("A compactor is already running"),
        "{out}"
    );
}
