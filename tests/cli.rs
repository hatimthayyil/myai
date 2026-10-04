use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
    process::{Command, Output, Stdio},
    time::SystemTime,
};

use ai::memory::{Memory, REC, REF, Store, Summary, level, seg_path};
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

    fn memories(&self) -> Vec<Memory> {
        let mut c = Command::new("git");
        self.isolate(&mut c);
        let blob = c
            .arg(format!("--git-dir={}", self.store().display()))
            .args(["cat-file", "blob", &format!("{REF}:log/0000/00")])
            .output()
            .unwrap()
            .stdout;
        assert_eq!(blob.len() % REC, 0);
        blob.chunks(REC)
            .map(|r| Memory::decode(r).unwrap())
            .collect()
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

    for i in 0..2 {
        s.ai(&["note", &format!("memory {i}")]);
    }
    assert!(s.ai(&["config", "WAKE_LINES=1"]).status.success());
    let r = s.ai(&["wake"]);
    assert_eq!(r.status.code(), Some(1));
    let out = text(&r.stdout);
    assert!(out.starts_with("Cannot wake") && out.contains("Run: ai memory nap 0-1@"));
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
    assert!(all.windows(2).all(|w| w[0].key() < w[1].key()));
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
    let origin = s.store_git(&["config", "ai.memory.origin"]);
    assert!(origin.len() == 6 && a.origin == origin && b.origin == origin);
    assert!(a.ts.len() == 16 && a.ts.ends_with('Z') && a.key() < b.key());

    s.git(&work, &["remote", "remove", "origin"]);
    s.cmd(&work, Some(&s.store()))
        .args(["note", "noted in a repo with no remote"])
        .output()
        .unwrap();
    assert_eq!(s.memories()[2].place.repo, "work");

    let show = text(&s.ai(&["show", "0"]).stdout);
    for f in [
        format!("origin  {origin}\n"),
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
            "#0 {}-{}-{} {}:{} {origin} acme/widget noted inside a repo",
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
    assert_eq!(heads(&["--repo", "acme/widget"]), ["#0", "1"]);
    assert_eq!(heads(&["--repo", "work"]), ["#2", "1"]);
    assert_eq!(heads(&["--agent", "pi"]), ["#1", "1"]);
    assert_eq!(heads(&["--session", "SESS-1"]), ["#0", "1"]);
    assert_eq!(
        heads(&["--origin", &origin.to_lowercase()]),
        ["#0", "#1", "#2", "3"]
    );
    assert_eq!(
        heads(&["--since", "2000-01-01", "--agent", "claude-code"]),
        ["#0", "1"]
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
    s.ai(&["nap", "0-1", "both"]);
    s.ai(&["forget", "0-1"]);
    s.ai(&["nap"]);
    s.ai(&["wake"]);
    assert_eq!(
        s.store_git(&["log", "--format=%s|%cn <%ce>", REF]),
        "forget 0-1|ai memory <ai@localhost>\n\
         nap 0-1|ai memory <ai@localhost>\n\
         note|ai memory <ai@localhost>\n\
         import|ai memory <ai@localhost>"
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
        vec!["nap"],
        vec!["nap", "0-1", "both"],
        vec!["wake"],
        vec!["config", "WAKE_LINES=4"],
        vec!["grep", "first", "-t"],
        vec!["zoom", "0-1", "--depth", "1"],
        vec!["show", "0-1"],
        vec!["forget", "0-1"],
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

fn day(i: u32) -> String {
    format!("2020-{:02}-{:02}", i / 28 + 1, i % 28 + 1)
}

fn nap_offer(out: &str) -> Option<String> {
    let at = out.find("Run: ai memory nap ")? + "Run: ai memory nap ".len();
    out[at..].split(' ').next().map(str::to_string)
}

impl Sandbox {
    fn remote(&self) -> PathBuf {
        self.path().join("remote.git")
    }

    fn clone(&self, name: &str) -> PathBuf {
        let r = self.remote();
        if !r.exists() {
            self.git(self.path(), &["init", "-q", "--bare", r.to_str().unwrap()]);
        }
        let d = self.path().join(name);
        self.say(&d, &["init"]);
        self.git_at(&d, &["remote", "add", "origin", r.to_str().unwrap()]);
        d
    }

    fn say(&self, d: &Path, args: &[&str]) -> String {
        let r = self.ai_at(d, args);
        assert!(r.status.success(), "{args:?}: {}", text(&r.stderr));
        text(&r.stdout)
    }

    fn import(&self, d: &Path, days: impl IntoIterator<Item = u32>, tag: &str) {
        let f = self.path().join("seed.txt");
        let lines: String = days
            .into_iter()
            .map(|i| format!("{} {tag} {i}\n", day(i)))
            .collect();
        fs::write(&f, lines).unwrap();
        self.say(d, &["import", f.to_str().unwrap()]);
    }

    fn settle(&self, d: &Path, summary: &str) -> usize {
        let mut n = 0;
        while let Some(id) = nap_offer(&self.say(d, &["nap"])) {
            self.say(d, &["nap", &id, summary]);
            n += 1;
        }
        n
    }

    fn rev(&self, d: &Path, spec: &str) -> String {
        self.git_at(d, &["rev-parse", spec])
    }

    fn texts(&self, d: &Path) -> Vec<String> {
        let s = Store::open(d).unwrap();
        let snap = s.snapshot().unwrap();
        let n = snap.log_len().unwrap();
        snap.log_slice(0, n)
            .unwrap()
            .into_iter()
            .map(|m| m.text)
            .collect()
    }
}

fn levels(d: &Path) -> Vec<u64> {
    let s = Store::open(d).unwrap();
    let snap = s.snapshot().unwrap();
    let n = snap.log_len().unwrap();
    let mut out = Vec::new();
    let mut size = 2;
    while size <= n {
        let built = snap.level_len(size).unwrap();
        for k in 0..built {
            let seg = snap.read(&seg_path(&level(size), k / 256)).unwrap();
            let at = (k % 256) as usize * REC;
            let sum = Summary::decode(&seg[at..at + REC]).unwrap();
            assert_eq!(
                sum.fp,
                snap.fingerprint(k * size, (k + 1) * size).unwrap(),
                "summary {k} of size {size}"
            );
        }
        if let Some(&below) = out.last() {
            assert!(built <= below / 2, "level {size} outgrew its halves");
        }
        out.push(built);
        size *= 2;
    }
    out
}

#[test]
fn sync_without_a_remote_stays_local() {
    let s = Sandbox::new();
    s.ai(&["init"]);
    s.ai(&["note", "alone"]);
    let out = s.say(&s.store(), &["sync"]);
    assert!(
        out.starts_with("No remote: this memory is local only. To sync it, run: git --git-dir "),
        "{out}"
    );
    let out = s.say(&s.store(), &["wake"]);
    assert!(
        !out.contains("Warning") && out.ends_with("You are awake.\n"),
        "{out}"
    );
    assert_eq!(s.store_git(&["for-each-ref", "--format=%(refname)"]), REF);
}

#[test]
fn sync_fast_forwards_and_is_idempotent() {
    let s = Sandbox::new();
    let (a, b) = (s.clone("a"), s.clone("b"));
    assert_eq!(s.say(&a, &["sync"]), "Up to date with origin.\n");
    for t in ["one", "two", "three"] {
        s.say(&a, &["note", t]);
    }
    assert_eq!(s.say(&a, &["sync"]), "Pushed to origin.\n");
    assert_eq!(
        s.say(&b, &["sync"]),
        "Merged 3 memories from origin; 1 summary to redo. Run: ai memory nap\n"
    );
    let head = s.rev(&a, REF);
    assert_eq!(s.rev(&b, REF), head);
    for d in [&b, &a, &b] {
        assert_eq!(s.say(d, &["sync"]), "Up to date with origin.\n");
    }
    assert_eq!(
        (s.rev(&a, REF), s.rev(&b, REF)),
        (head.clone(), head.clone())
    );
    assert_eq!(s.rev(&s.remote(), REF), head);
    assert_eq!(s.texts(&b), ["one", "two", "three"]);
}

#[test]
fn unrelated_interleaved_clones_converge() {
    let s = Sandbox::new();
    let (a, b) = (s.clone("a"), s.clone("b"));
    s.import(&a, (0..40).map(|i| 2 * i), "a");
    s.import(&b, (0..40).map(|i| 2 * i + 1), "b");
    s.settle(&a, "napped by a");
    s.settle(&b, "napped by b");
    assert_ne!(
        s.git_at(&a, &["config", "ai.memory.origin"]),
        s.git_at(&b, &["config", "ai.memory.origin"])
    );
    assert_eq!(s.say(&a, &["sync"]), "Pushed to origin.\n");
    let out = s.say(&b, &["sync"]);
    assert!(
        out.starts_with("Merged 40 memories from origin; positions from #0 renumbered; 78 summaries to redo. Run: ai memory nap\n")
            && out.ends_with("Pushed to origin.\n"),
        "{out}"
    );
    let parents = s.git_at(&b, &["rev-list", "--parents", "-n1", REF]);
    assert_eq!(parents.split(' ').count(), 3);
    assert_eq!(
        s.git_at(&b, &["log", "-1", "--format=%s", REF]),
        "merge origin"
    );
    let out = s.say(&a, &["sync"]);
    assert!(
        out.starts_with(
            "Merged 40 memories from origin; positions from #1 renumbered; 78 summaries"
        ),
        "{out}"
    );
    assert_eq!(s.rev(&a, REF), s.rev(&b, REF));
    let want: Vec<_> = (0..80)
        .map(|i| format!("{} {i}", if i % 2 == 0 { "a" } else { "b" }))
        .collect();
    assert_eq!(s.texts(&a), want);
    assert_eq!(levels(&a), [0; 6]);

    assert_eq!(s.settle(&a, "renapped by a"), 78);
    assert_eq!(s.settle(&b, "renapped by b"), 78);
    for d in [&a, &b, &a] {
        s.say(d, &["sync"]);
    }
    let tree = s.rev(&a, &format!("{REF}^{{tree}}"));
    assert_eq!(s.rev(&b, &format!("{REF}^{{tree}}")), tree);
    assert_eq!(s.rev(&s.remote(), &format!("{REF}^{{tree}}")), tree);
    assert_eq!(levels(&b), [40, 20, 10, 5, 2, 1]);
    assert_eq!(s.say(&b, &["nap"]), "Nothing left to compress.\n");
    assert_eq!(s.say(&b, &["sync"]), "Up to date with origin.\n");
}

#[test]
fn summaries_are_reused_at_shifted_offsets() {
    let s = Sandbox::new();
    let (a, b) = (s.clone("a"), s.clone("b"));
    s.import(&a, 10..18, "a");
    s.import(&b, 0..4, "b");
    assert_eq!(s.settle(&a, "napped by a"), 7);
    assert_eq!(s.settle(&b, "napped by b"), 3);
    s.say(&a, &["sync"]);
    assert_eq!(
        s.say(&b, &["sync"]),
        "Merged 8 memories from origin; 1 summary to redo. Run: ai memory nap\nPushed to origin.\n"
    );
    assert_eq!(levels(&b), [6, 3, 0]);
    let by = |id| {
        let out = s.say(&b, &["show", id]);
        out.lines().last().unwrap().to_string()
    };
    assert_eq!(
        ["0-3", "4-7", "8-9", "10-11"].map(by),
        ["b", "a", "a", "a"].map(|w| format!("text    napped by {w}"))
    );
    assert!(s.say(&b, &["nap"]).contains("Run: ai memory nap 0-7@"));
    let out = s.say(&a, &["sync"]);
    assert!(
        out.starts_with("Merged 4 memories from origin; positions from #0 renumbered; 1 summary"),
        "{out}"
    );
}

#[test]
fn an_unreachable_remote_is_a_warning() {
    let s = Sandbox::new();
    let (a, b) = (s.clone("a"), s.clone("b"));
    s.say(&a, &["note", "first"]);
    s.say(&a, &["note", "second"]);
    s.say(&a, &["sync"]);
    s.git_at(
        &b,
        &[
            "fetch",
            "-q",
            "origin",
            "+refs/ai/memory:refs/ai/remotes/origin/memory",
        ],
    );
    fs::rename(s.remote(), s.path().join("gone.git")).unwrap();
    let out = s.say(&b, &["sync"]);
    assert!(
        out.starts_with("Warning: cannot reach origin: ")
            && out.ends_with(
                "Merged 2 memories from origin; 1 summary to redo. Run: ai memory nap\n"
            ),
        "{out}"
    );
    s.say(&b, &["note", "third"]);
    let out = s.say(&b, &["wake"]);
    assert!(
        out.starts_with("Warning: cannot reach origin: ") && out.contains("You are awake."),
        "{out}"
    );
    assert_eq!(s.texts(&b), ["first", "second", "third"]);
}

#[test]
fn wake_gives_up_on_a_slow_remote() {
    let s = Sandbox::new();
    let a = s.clone("a");
    s.git_at(
        &a,
        &[
            "remote",
            "set-url",
            "origin",
            "ssh://example.invalid/memory",
        ],
    );
    s.say(&a, &["note", "patience"]);
    let t0 = std::time::Instant::now();
    let r = s
        .cmd(s.path(), Some(&a))
        .arg("wake")
        .env("GIT_SSH_COMMAND", "sh -c 'sleep 10' --")
        .output()
        .unwrap();
    assert!(t0.elapsed().as_secs() < 8, "{:?}", t0.elapsed());
    let out = text(&r.stdout);
    assert!(r.status.success(), "{}", text(&r.stderr));
    assert!(
        out.starts_with("Warning: cannot reach origin: no answer in time.\n")
            && out.contains("#0 ")
            && out.ends_with("You are awake.\n"),
        "{out}"
    );
}

#[test]
fn wake_takes_the_remote_first() {
    let s = Sandbox::new();
    let (a, b) = (s.clone("a"), s.clone("b"));
    s.say(&a, &["note", "from a"]);
    s.say(&a, &["sync"]);
    let out = s.say(&b, &["wake"]);
    assert!(
        out.starts_with("Merged 1 memory from origin.\n") && out.contains("from a"),
        "{out}"
    );
    s.say(&b, &["note", "from b"]);
    let out = s.say(&b, &["wake"]);
    assert!(!out.contains("Merged") && !out.contains("Pushed"), "{out}");
    assert_eq!(s.rev(&s.remote(), REF), s.rev(&b, REF));
}

#[test]
fn a_lost_push_race_is_retried() {
    use std::os::unix::fs::PermissionsExt;
    let s = Sandbox::new();
    let (a, b) = (s.clone("a"), s.clone("b"));
    s.say(&b, &["note", "from b"]);
    s.git_at(&b, &["push", "-q", "origin", "refs/ai/memory:refs/ai/held"]);
    let hook = s.remote().join("hooks/pre-receive");
    fs::write(
        &hook,
        "#!/bin/sh\n\
         if [ -f armed ]; then\n\
           rm armed\n\
           unset GIT_QUARANTINE_PATH GIT_OBJECT_DIRECTORY GIT_ALTERNATE_OBJECT_DIRECTORIES\n\
           git update-ref refs/ai/memory refs/ai/held\n\
         fi\n",
    )
    .unwrap();
    fs::set_permissions(&hook, fs::Permissions::from_mode(0o755)).unwrap();
    fs::write(s.remote().join("armed"), "").unwrap();
    s.say(&a, &["note", "from a"]);
    let out = s.say(&a, &["sync"]);
    assert!(!s.remote().join("armed").exists());
    assert!(
        out.starts_with("Merged 1 memory from origin") && out.ends_with("Pushed to origin.\n"),
        "{out}"
    );
    assert_eq!(s.rev(&s.remote(), REF), s.rev(&a, REF));
    let mut texts = s.texts(&a);
    texts.sort();
    assert_eq!(texts, ["from a", "from b"]);
}

#[test]
fn nap_refuses_a_block_a_sync_changed() {
    let s = Sandbox::new();
    let (a, b) = (s.clone("a"), s.clone("b"));
    s.import(&b, [0], "b");
    s.say(&b, &["sync"]);
    s.import(&a, [1, 2], "a");
    let id = nap_offer(&s.say(&a, &["nap"])).unwrap();
    assert!(id.starts_with("0-1@") && id.len() == 8, "{id}");
    s.say(&a, &["sync"]);
    let r = s.ai_at(&a, &["nap", &id, "stale"]);
    assert_eq!(r.status.code(), Some(1));
    assert_eq!(
        text(&r.stderr),
        "0-1: block changed by a sync. Run: ai memory nap\n"
    );
    let fresh = nap_offer(&s.say(&a, &["nap"])).unwrap();
    assert_ne!(fresh, id);
    assert!(
        s.say(&a, &["nap", &fresh, "fresh"])
            .starts_with("0-1 saved.")
    );
    assert!(!s.ai_at(&a, &["nap", "0-1@zz", "x"]).status.success());
}
