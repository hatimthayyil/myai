use std::{
    fs,
    path::Path,
    process::{Command, Output, Stdio},
};

use tempfile::TempDir;

fn ai(cwd: &Path, store: Option<&Path>, args: &[&str]) -> Output {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_ai"));
    cmd.current_dir(cwd)
        .arg("memory")
        .args(args)
        .env_remove("AI_MEMORY_DIR");
    if let Some(d) = store {
        cmd.env("AI_MEMORY_DIR", d);
    }
    cmd.output().unwrap()
}

fn text(b: &[u8]) -> String {
    String::from_utf8_lossy(b).into_owned()
}

fn outside_repo() -> TempDir {
    let tmp = TempDir::new().unwrap();
    assert!(
        !tmp.path().ancestors().any(|a| a.join(".git").exists()),
        "the temp dir is inside a git repository"
    );
    tmp
}

#[test]
fn defaults_to_the_repo_root() {
    let tmp = outside_repo();
    fs::create_dir(tmp.path().join(".git")).unwrap();
    let sub = tmp.path().join("deep/er");
    fs::create_dir_all(&sub).unwrap();
    let r = ai(&sub, None, &["init"]);
    assert!(r.status.success(), "{}", text(&r.stderr));
    assert!(tmp.path().join(".ai/memory/config").exists());
    let r = ai(&sub, None, &["wake"]);
    assert!(r.status.success() && text(&r.stdout).contains("You are awake."));
}

#[test]
fn outside_a_repo_it_refuses() {
    let tmp = outside_repo();
    let r = ai(tmp.path(), None, &["init"]);
    assert_eq!(r.status.code(), Some(1));
    assert_eq!(
        text(&r.stderr).trim_end(),
        "Not in a git repository. Run inside a repo, or point AI_MEMORY_DIR at a memory."
    );
    assert_eq!(fs::read_dir(tmp.path()).unwrap().count(), 0);
}

#[test]
fn exit_codes_and_streams() {
    let tmp = outside_repo();
    let d = tmp.path().join("m");
    let r = ai(tmp.path(), Some(&d), &["wake"]);
    assert_eq!(r.status.code(), Some(1));
    assert!(text(&r.stderr).contains("No memory at") && text(&r.stderr).contains("ai memory init"));
    assert!(!d.exists());

    assert!(ai(tmp.path(), Some(&d), &["init"]).status.success());
    let r = ai(tmp.path(), Some(&d), &["wake"]);
    assert!(r.status.success() && text(&r.stdout).contains("No memories yet"));
    assert!(!ai(tmp.path(), Some(&d), &["zoom"]).status.success());

    for i in 0..2 {
        ai(tmp.path(), Some(&d), &["note", &format!("memory {i}")]);
    }
    fs::write(d.join("config"), "WAKE_LINES = 1\n").unwrap();
    let r = ai(tmp.path(), Some(&d), &["wake"]);
    assert_eq!(r.status.code(), Some(1));
    let out = text(&r.stdout);
    assert!(
        out.starts_with("Cannot wake") && out.contains("Run: ai memory nap 0-1 \"<your line>\"")
    );
}

#[test]
fn parallel_processes_get_distinct_ids() {
    let tmp = outside_repo();
    let d = tmp.path().join("m");
    assert!(ai(tmp.path(), Some(&d), &["init"]).status.success());
    let p = 16;
    let kids: Vec<_> = (0..p)
        .map(|i| {
            Command::new(env!("CARGO_BIN_EXE_ai"))
                .args(["memory", "note", &format!("parallel note {i}")])
                .env("AI_MEMORY_DIR", &d)
                .stdout(Stdio::null())
                .spawn()
                .unwrap()
        })
        .collect();
    for mut k in kids {
        assert!(k.wait().unwrap().success());
    }
    let log = fs::read(d.join("LOG.txt")).unwrap();
    let mut ids: Vec<_> = log
        .chunks(320)
        .map(|r| text(r).split(' ').next().unwrap().to_string())
        .collect();
    ids.sort();
    let mut want: Vec<_> = (0..p).map(|i| format!("#{i}")).collect();
    want.sort();
    assert_eq!(ids, want);
}

#[test]
fn init_ignores_the_repo_store() {
    let tmp = outside_repo();
    let root = tmp.path();
    fs::create_dir(root.join(".git")).unwrap();
    let ignore = root.join(".gitignore");

    let r = ai(root, None, &["init"]);
    assert!(
        text(&r.stdout).contains("Added /.ai/memory/ to "),
        "{}",
        text(&r.stdout)
    );
    assert_eq!(fs::read_to_string(&ignore).unwrap(), "/.ai/memory/\n");

    let r = ai(root, None, &["init"]);
    assert!(!text(&r.stdout).contains("Added"));
    assert_eq!(fs::read_to_string(&ignore).unwrap(), "/.ai/memory/\n");

    fs::write(&ignore, "target").unwrap();
    assert!(text(&ai(root, None, &["init"]).stdout).contains("Added"));
    assert_eq!(
        fs::read_to_string(&ignore).unwrap(),
        "target\n/.ai/memory/\n"
    );

    fs::write(&ignore, "  .ai/memory  \n").unwrap();
    assert!(!text(&ai(root, None, &["init"]).stdout).contains("Added"));
    assert_eq!(fs::read_to_string(&ignore).unwrap(), "  .ai/memory  \n");
}

#[test]
fn init_with_an_explicit_dir_leaves_gitignore_alone() {
    let tmp = outside_repo();
    let root = tmp.path();
    fs::create_dir(root.join(".git")).unwrap();
    let r = ai(root, Some(&root.join("m")), &["init"]);
    assert!(r.status.success() && !text(&r.stdout).contains("Added"));
    assert!(!root.join(".gitignore").exists());
}

#[test]
fn a_closed_pipe_is_quiet() {
    let tmp = outside_repo();
    let d = tmp.path().join("m");
    ai(tmp.path(), Some(&d), &["init"]);
    let seed = tmp.path().join("seed.txt");
    let lines: String = (0..5000)
        .map(|i| format!("2020-01-01 memory number {i} padded to make the output long enough\n"))
        .collect();
    fs::write(&seed, lines).unwrap();
    ai(tmp.path(), Some(&d), &["import", seed.to_str().unwrap()]);
    let mut child = Command::new(env!("CARGO_BIN_EXE_ai"))
        .args(["memory", "recall", "memory"])
        .env("AI_MEMORY_DIR", &d)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    drop(child.stdout.take());
    let r = child.wait_with_output().unwrap();
    assert!(r.status.success(), "{:?}", r.status);
    assert_eq!(text(&r.stderr), "");
}
