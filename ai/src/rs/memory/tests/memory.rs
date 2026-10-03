use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
    process::ExitCode,
    sync::LazyLock,
    thread,
};

use ai_memory::{Cli, Knob, LOG_REC, Location, Store, TREE_REC, pending, pending_count};
use clap::Parser;
use regex::Regex;
use tempfile::TempDir;

const N: u64 = 2000;
const CAP_CHARS: usize = 30000;
const CAP_LINES: usize = 2000;

struct Out {
    code: u8,
    stdout: String,
    stderr: String,
}

fn run(dir: &Path, args: &[&str]) -> Out {
    let cli = match Cli::try_parse_from(std::iter::once("memory").chain(args.iter().copied())) {
        Ok(cli) => cli,
        Err(e) => {
            return Out {
                code: 2,
                stdout: String::new(),
                stderr: e.to_string(),
            };
        }
    };
    let mut buf = Vec::new();
    let result = cli.run(
        &Location {
            dir: dir.to_path_buf(),
            repo: None,
        },
        &mut buf,
    );
    let stdout = String::from_utf8(buf).unwrap();
    match result {
        Ok(c) => Out {
            code: u8::from(c != ExitCode::SUCCESS),
            stdout,
            stderr: String::new(),
        },
        Err(e) => Out {
            code: 1,
            stdout,
            stderr: e.to_string(),
        },
    }
}

fn nap_id(out: &str) -> Option<String> {
    static RE: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"ai memory nap (\d+)-(\d+)").unwrap());
    RE.captures(out).map(|c| format!("{}-{}", &c[1], &c[2]))
}

fn offered(out: &str) -> Vec<&str> {
    out.lines()
        .filter(|l| l.contains("ai memory nap ") || l.contains("ai memory wake "))
        .collect()
}

fn complete(t: u64) -> Vec<(u64, u64)> {
    let (mut out, mut size) = (Vec::new(), 2);
    while size <= t {
        out.extend((0..t / size).map(|i| (i * size, (i + 1) * size)));
        size *= 2;
    }
    out
}

fn settle(dir: &Path, text: &str) -> usize {
    let mut n = 0;
    while let Some(id) = nap_id(&run(dir, &["nap"]).stdout) {
        assert_eq!(run(dir, &["nap", &id, text]).code, 0, "nap {id} rejected");
        n += 1;
    }
    n
}

fn store() -> (TempDir, PathBuf) {
    let tmp = TempDir::new().unwrap();
    let d = tmp.path().join("memory");
    assert_eq!(run(&d, &["init"]).code, 0);
    fs::remove_file(d.join("config")).unwrap();
    (tmp, d)
}

fn treesize(d: &Path) -> u64 {
    fs::read_dir(d.join("TREE"))
        .unwrap()
        .map(|e| e.unwrap().metadata().unwrap().len())
        .sum()
}

fn log_size(d: &Path) -> u64 {
    fs::metadata(d.join("LOG.txt")).unwrap().len()
}

fn fingerprint(d: &Path) -> BTreeMap<PathBuf, Vec<u8>> {
    let mut out = BTreeMap::new();
    let mut stack = vec![d.to_path_buf()];
    while let Some(p) = stack.pop() {
        for e in fs::read_dir(p).unwrap() {
            let p = e.unwrap().path();
            if p.is_dir() {
                stack.push(p);
            } else if p.file_name().unwrap() != ".lock" {
                out.insert(
                    p.strip_prefix(d).unwrap().to_path_buf(),
                    fs::read(&p).unwrap(),
                );
            }
        }
    }
    out
}

fn halves(d: &Path, id: &str) -> Vec<(u64, u64)> {
    let r = run(d, &["zoom", id]);
    assert_eq!(r.code, 0, "zoom {id} failed: {}", r.stderr);
    let re = Regex::new(r"^#(\d+)(?:-(\d+))? ").unwrap();
    r.stdout
        .lines()
        .map(|l| {
            let c = re
                .captures(l)
                .unwrap_or_else(|| panic!("zoom printed a line with no id: {l:?}"));
            let a: u64 = c[1].parse().unwrap();
            (
                a,
                c.get(2)
                    .map_or(a + 1, |m| m.as_str().parse::<u64>().unwrap() + 1),
            )
        })
        .collect()
}

#[test]
fn a_synthetic_life() {
    let (tmp, d) = store();
    let d = d.as_path();
    let wake_lines = Knob::WakeLines.default() as usize;

    let r = run(d, &["note", &"x".repeat(281)]);
    assert!(
        r.code == 1 && r.stderr.contains("Too long"),
        "over-long note accepted"
    );
    let r = run(d, &["note", "two\nlines"]);
    assert!(
        r.code == 1 && r.stderr.contains("one line"),
        "multi-line note accepted"
    );
    assert_eq!(run(d, &["note", "   "]).code, 1, "empty note accepted");
    let r = run(d, &["wake"]);
    assert!(r.stdout.contains("No memories yet"));
    assert!(r.stdout.trim_end().ends_with("You are awake."));

    let seed = tmp.path().join("seed.txt");
    let day = jiff::civil::date(2020, 1, 1);
    let lines: String = (0..N)
        .map(|i| {
            format!(
                "{} memory number {i}, a thing that happened, was weighed against the rest of the \
                 week, turned out to matter more than anyone guessed at the time, and left a mark \
                 on every plan that followed it\n",
                day.checked_add(jiff::Span::new().days((i / 5) as i64))
                    .unwrap()
            )
        })
        .collect();
    fs::write(&seed, lines).unwrap();
    let r = run(d, &["import", seed.to_str().unwrap()]);
    assert!(
        r.stdout.contains(&format!("Imported {N}")),
        "import failed: {}{}",
        r.stdout,
        r.stderr
    );
    assert!(
        !d.join("config").exists(),
        "a store wrote its own config file"
    );

    let r = run(d, &["wake"]);
    assert!(
        r.code == 1 && r.stdout.contains("Cannot wake"),
        "wake must refuse while work is pending"
    );
    assert!(r.stdout.contains("wake again"));

    let mut naps = 0;
    let mut r = run(d, &["nap"]);
    assert!(r.stdout.contains("Compress memories #"));
    while !r.stdout.contains("Nothing left to compress") {
        let line = offered(&r.stdout);
        assert!(
            !line.is_empty(),
            "no command offered:\n{}{}",
            r.stdout,
            r.stderr
        );
        assert!(
            line[0].starts_with("Run: "),
            "offered as a label: {:?}",
            line[0]
        );
        let id = nap_id(&r.stdout).unwrap();
        let body: Vec<_> = r
            .stdout
            .lines()
            .filter(|l| l.starts_with("  #"))
            .map(str::trim)
            .collect();
        let joined: String = body.join(" ").chars().take(280).collect();
        let text = match joined.trim() {
            "" => "empty",
            t => t,
        };
        r = run(d, &["nap", &id, text]);
        assert_eq!(r.code, 0, "nap rejected a valid merge: {}", r.stderr);
        naps += 1;
    }
    assert!(
        !r.stdout.contains("You are awake"),
        "nap must never claim the agent is awake"
    );
    assert_eq!(naps, complete(N).len());
    assert_eq!(
        run(d, &["wake"]).code,
        0,
        "wake still refuses after a full nap chain"
    );

    let mut parts = Vec::new();
    for k in 1.. {
        let r = run(d, &["wake", &k.to_string()]);
        if r.code != 0 {
            break;
        }
        assert!(
            r.stdout.len() < CAP_CHARS,
            "part {k} is {} chars",
            r.stdout.len()
        );
        assert!(
            r.stdout.lines().count() < CAP_LINES,
            "part {k} is over {CAP_LINES} lines"
        );
        parts.push(
            r.stdout
                .lines()
                .filter(|l| l.starts_with('#'))
                .map(String::from)
                .collect::<Vec<_>>(),
        );
    }
    assert!(
        parts.len() > 1,
        "a {wake_lines}-line memory should need more than one part"
    );
    let lines: Vec<_> = parts.concat();
    assert_eq!(lines.len(), wake_lines);
    assert!(
        lines[lines.len() - 1].starts_with(&format!("#{} ", N - 1)),
        "newest memory not last / not raw"
    );
    assert!(
        lines[0].starts_with("#0-"),
        "oldest line should be a summary block"
    );
    assert!(
        Regex::new(r"Run: ai memory wake 2")
            .unwrap()
            .is_match(&run(d, &["wake"]).stdout)
    );
    assert!(
        run(d, &["wake", &parts.len().to_string()])
            .stdout
            .contains("You are awake.")
    );
    assert_eq!(run(d, &["wake", &(parts.len() + 1).to_string()]).code, 1);

    let logsz = log_size(d);
    run(d, &["note", "one more thing happened today"]);
    assert!(log_size(d) > logsz, "note did not append");
    assert_eq!(logsz % LOG_REC, 0);
    for e in fs::read_dir(d.join("TREE")).unwrap() {
        assert_eq!(e.unwrap().metadata().unwrap().len() % TREE_REC, 0);
    }

    let r = run(d, &["nap", "0-1", "attempted overwrite"]);
    assert!(r.code == 0 && r.stdout.contains("Nothing left to compress"));

    let r = run(d, &["recall", "memory number 7,"]);
    assert!(
        r.code == 0 && r.stdout.contains("#7 "),
        "recall missed a memory"
    );
    assert!(r.stdout.contains("1 match."), "{}", r.stdout);
    assert!(
        run(d, &["recall", "^#7 "])
            .stdout
            .contains("memory number 7,")
    );
    let r = run(d, &["recall", "2020-01-02"]);
    assert!(
        r.stdout.contains("#7 ") && r.stdout.contains("5 matches."),
        "{}",
        r.stdout
    );

    let (target, mut lo, mut hi, mut calls) = (777, 0, 1024, 0);
    while hi - lo > 1 {
        let mid = (lo + hi) / 2;
        let kids = halves(d, &format!("{lo}-{}", hi - 1));
        assert_eq!(kids, vec![(lo, mid), (mid, hi)]);
        (lo, hi) = kids[usize::from(target >= mid)];
        calls += 1;
    }
    assert!(
        lo == target && calls == 10,
        "halving took {calls} calls and landed on #{lo}"
    );
    assert!(
        run(d, &["zoom", "776-777"])
            .stdout
            .contains(&format!("memory number {target},"))
    );

    let r = run(d, &["zoom", "1024-2047"]);
    assert!(
        r.stdout.contains("#1536-2047 not compressed yet"),
        "{}",
        r.stdout
    );
    let r = run(d, &["zoom", &format!("{N}-{}", N + 1)]);
    assert!(
        r.stdout.matches('\n').count() == 1 && r.stdout.contains(&format!("#{N} ")),
        "{}",
        r.stdout
    );

    assert_eq!(
        run(d, &["zoom", "3-9"]).code,
        1,
        "zoom accepted a non-block"
    );
    assert_eq!(
        run(d, &["zoom", "9-3"]).code,
        1,
        "zoom accepted a backwards range"
    );
    assert_ne!(run(d, &["zoom"]).code, 0, "zoom with no id must show usage");
    let r = run(d, &["zoom", "1048576-2097151"]);
    assert!(
        r.code == 1
            && r.stderr.contains("beyond the memory")
            && r.stderr.contains("ai memory wake")
    );

    let (before, logsize) = (treesize(d), log_size(d));
    let r = run(d, &["forget", "16-31"]);
    assert!(r.stdout.contains("16-31"), "{}{}", r.stdout, r.stderr);
    assert!(treesize(d) < before, "forget did not shrink the tree");
    assert_eq!(log_size(d), logsize, "forget touched the log");
    assert_eq!(
        run(d, &["wake"]).code,
        1,
        "wake should refuse after a forget"
    );
    let mid = treesize(d);
    let r = run(d, &["nap", "0-1", "attempted overwrite"]);
    assert!(
        r.code == 0 && r.stdout.contains("already settled"),
        "{}",
        r.stderr
    );
    assert_eq!(
        treesize(d),
        mid,
        "resubmitting a settled block wrote something"
    );
    let r = run(d, &["nap", "0-31", "out of order"]);
    assert!(r.code == 1 && r.stderr.contains("Wrong block"));
    assert!(
        settle(d, "rebuilt after forget") > 0,
        "forget created no work"
    );
    assert_eq!(run(d, &["wake"]).code, 0);
    assert_eq!(
        treesize(d),
        before,
        "tree did not return to its original size"
    );
    assert_eq!(run(d, &["forget", "17-32"]).code, 1);
    run(d, &["forget", "16-31"]);
    let z = run(d, &["zoom", "0-31"]).stdout;
    assert!(z.contains("#16-31 not compressed yet"), "{z}");
    settle(d, "rebuilt after forget");
    assert_eq!(treesize(d), before);
    assert_eq!(run(d, &["forget", "1048576-1048577"]).code, 1);

    run(
        d,
        &[
            "note",
            "reunião com João em São Paulo: ação aprovada, coração tranquilo",
        ],
    );
    run(
        d,
        &["note", "a plain ascii memory right after the accented one"],
    );
    assert!(run(d, &["recall", "coração"]).stdout.contains("João"));
    assert!(
        run(d, &["recall", "plain ascii memory right after"])
            .stdout
            .contains(&format!("#{} ", N + 2))
    );
    let r = run(d, &["note", &"ã".repeat(150)]);
    assert!(
        r.code == 1 && r.stderr.contains("300 bytes"),
        "{}",
        r.stderr
    );

    settle(d, "settled");
    assert_eq!(run(d, &["wake"]).code, 0);

    let t0 = log_size(d) / LOG_REC;
    let before = run(d, &["wake", "1", &t0.to_string()]);
    assert_eq!(before.code, 0, "{}{}", before.stdout, before.stderr);
    run(d, &["note", "a note that lands between two wake calls"]);
    assert_eq!(
        run(d, &["wake", "1", &t0.to_string()]).stdout,
        before.stdout
    );
    assert_eq!(run(d, &["wake", "1", &(t0 + 99).to_string()]).code, 1);

    settle(d, "settled mid-wake");
    let r = run(d, &["wake", "1", &t0.to_string()]);
    assert!(
        r.code == 0 && r.stdout == before.stdout,
        "{}{}",
        r.stdout,
        r.stderr
    );
    let s = Store::open(d).unwrap();
    for t in (1..40).chain([t0 - 1, t0, t0 + 1]) {
        assert_eq!(
            pending_count(&s, t).unwrap(),
            pending(&s, t, None).unwrap().len() as u64,
            "pending_count disagrees with pending at T={t}"
        );
    }

    let r = run(d, &["recall", "memory number"]);
    assert!(r.stdout.len() < CAP_CHARS);
    assert!(r.stdout.contains("Narrow the regex"));

    let r = run(d, &["config", "WAKE_LINES=12"]);
    assert!(
        r.stdout.contains("12") && r.stdout.contains("default 96"),
        "{}",
        r.stdout
    );
    assert!(
        run(d, &["wake"]).stdout.lines().count() <= 13,
        "wake ignored the new size"
    );
    let r = run(d, &["config", "WAKE_LINES="]);
    assert!(!r.stdout.contains("default"));
    assert!(
        run(d, &["wake"]).stdout.lines().count() > 13,
        "the default did not come back"
    );
    for bad in [
        "WAKE_LINES=0",
        "WAKE_LINES=x",
        "ENTRY_CHARS=999",
        "NOPE=1",
        "WAKE_LINES",
    ] {
        assert_eq!(run(d, &["config", bad]).code, 1, "config accepted {bad}");
    }

    let mut cfg = fs::read_to_string(d.join("config")).unwrap();
    cfg += "WAKE_LINES=12\n";
    fs::write(d.join("config"), cfg).unwrap();
    let before = fingerprint(d);
    assert!(before.len() > 3 && !before[Path::new("LOG.txt")].is_empty());
    for _ in 0..3 {
        let r = run(d, &["init"]);
        assert!(r.code == 0 && r.stdout.contains("Found"));
    }
    assert_eq!(fingerprint(d), before, "init modified an existing memory");
    assert!(
        run(d, &["wake"])
            .stdout
            .trim_end()
            .ends_with("You are awake.")
    );
}

#[test]
fn a_missing_dir_is_reported_not_created() {
    let tmp = TempDir::new().unwrap();
    let ghost = tmp.path().join("typo");
    let r = run(&ghost, &["wake"]);
    assert!(
        r.code == 1 && r.stderr.contains("No memory at") && r.stderr.contains("ai memory init")
    );
    assert!(!ghost.exists());
}

#[test]
fn init_prints_the_block_and_is_idempotent() {
    let tmp = TempDir::new().unwrap();
    let d = tmp.path().join("m");
    let r = run(&d, &["init"]);
    assert!(r.code == 0 && r.stdout.contains("## Memory") && r.stdout.contains("You are a"));
    assert!(r.stdout.contains("Don't run ai memory.`"));
    assert!(r.stdout.contains("this repository's memory."));
    assert!(!r.stdout.contains("OptMem") && !r.stdout.contains("machine"));
    assert!(
        !fs::read_to_string(d.join("config"))
            .unwrap()
            .contains("OptMem")
    );
    assert!(d.join("config").exists());
    assert!(run(&d, &["init"]).stdout.contains("Found"));
    assert!(run(&d, &["wake"]).stdout.contains("You are awake."));
}

#[test]
fn a_bad_config_names_its_line() {
    let (_tmp, d) = store();
    fs::write(d.join("config"), "WAKE_LNES = 100\n").unwrap();
    for c in ["wake", "config"] {
        let r = run(&d, &[c]);
        assert!(
            r.code == 1 && r.stderr.contains("config line 1") && r.stderr.contains("WAKE_LNES"),
            "{}",
            r.stderr
        );
    }
}

#[test]
fn filesystem_errors_speak_plainly() {
    let tmp = TempDir::new().unwrap();
    let file = tmp.path().join("file");
    fs::write(&file, "").unwrap();
    let r = run(&file, &["init"]);
    assert!(
        r.code == 1 && r.stderr.contains("Not a directory"),
        "{}",
        r.stderr
    );
}

#[test]
fn parallel_notes_get_distinct_ids() {
    let (_tmp, d) = store();
    let p = 16;
    thread::scope(|sc| {
        for i in 0..p {
            let d = &d;
            sc.spawn(move || run(d, &["note", &format!("parallel note {i}")]));
        }
    });
    let log = fs::read(d.join("LOG.txt")).unwrap();
    let mut ids: Vec<_> = log
        .chunks(LOG_REC as usize)
        .map(|r| {
            String::from_utf8_lossy(r)
                .split(' ')
                .next()
                .unwrap()
                .to_string()
        })
        .collect();
    ids.sort();
    let mut want: Vec<_> = (0..p).map(|i| format!("#{i}")).collect();
    want.sort();
    assert_eq!(ids, want);

    let mut f = fs::OpenOptions::new()
        .append(true)
        .open(d.join("LOG.txt"))
        .unwrap();
    std::io::Write::write_all(
        &mut f,
        b"#99 2026-01-01 a half-written record killed by a power cut",
    )
    .unwrap();
    let r = run(&d, &["note", "the memory right after a torn write"]);
    assert_eq!(r.code, 0, "{}", r.stderr);
    assert_eq!(log_size(&d) % LOG_REC, 0);
    assert!(r.stdout.contains(&format!("Saved as #{p}")));
    assert!(
        run(&d, &["recall", "right after a torn write"])
            .stdout
            .contains(&format!("#{p} "))
    );

    settle(&d, "settled");
    assert!(
        run(&d, &["wake"])
            .stdout
            .trim_end()
            .ends_with("You are awake.")
    );
}

#[test]
fn a_blank_summary_points_at_forget() {
    let (tmp, d) = store();
    for i in 0..4 {
        run(&d, &["note", &format!("corrupt store memory {i}")]);
    }
    for (id, s) in [("0-1", "one"), ("2-3", "two"), ("0-3", "all")] {
        run(&d, &["nap", id, s]);
    }
    fs::write(d.join("config"), "WAKE_LINES = 2\n").unwrap();
    let blank = [vec![b' '; 287], vec![b'\n']].concat();
    let tree2 = d.join("TREE/2");
    let mut rec = fs::read(&tree2).unwrap();
    rec[..288].copy_from_slice(&blank);
    fs::write(&tree2, &rec).unwrap();
    let r = run(&d, &["wake"]);
    assert!(
        r.code == 1 && r.stderr.contains("forget 0-1"),
        "{}{}",
        r.stdout,
        r.stderr
    );

    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(&tree2, fs::Permissions::from_mode(0o000)).unwrap();
    if fs::read(&tree2).is_err() {
        let r = run(&d, &["wake"]);
        assert!(
            r.code == 1
                && r.stderr.contains("Permission denied")
                && !r.stdout.contains("not compressed")
        );
    }
    fs::set_permissions(&tree2, fs::Permissions::from_mode(0o644)).unwrap();

    let bad = tmp.path().join("bad.txt");
    fs::write(&bad, "2027-99-99 an impossible date\n").unwrap();
    let r = run(&d, &["import", bad.to_str().unwrap()]);
    assert!(
        r.code == 1 && r.stderr.contains("not a real date"),
        "{}",
        r.stderr
    );
}

#[test]
fn a_blank_half_points_at_forget() {
    let (_tmp, d) = store();
    for i in 0..32 {
        run(&d, &["note", &format!("half probe memory {i}")]);
    }
    loop {
        let r = run(&d, &["nap"]);
        if r.stdout.contains("Compress memories #0-31 ") {
            break;
        }
        run(&d, &["nap", &nap_id(&r.stdout).unwrap(), "settled"]);
    }
    let tree16 = d.join("TREE/16");
    let mut rec = fs::read(&tree16).unwrap();
    rec[..287].fill(b' ');
    fs::write(&tree16, &rec).unwrap();
    let r = run(&d, &["nap"]);
    assert!(
        r.code == 1 && r.stderr.contains("forget 0-15"),
        "{}{}",
        r.stdout,
        r.stderr
    );
}

#[test]
fn utf8_is_enforced() {
    let (tmp, d) = store();
    run(&d, &["note", "an arrow \u{2192} survives any locale"]);
    assert!(run(&d, &["wake"]).stdout.contains('\u{2192}'));

    let latin1 = tmp.path().join("latin1.txt");
    fs::write(&latin1, b"2027-01-01 caf\xe9 in latin-1\n").unwrap();
    let r = run(&d, &["import", latin1.to_str().unwrap()]);
    assert!(
        r.code == 1 && r.stderr.contains("not UTF-8"),
        "{}",
        r.stderr
    );

    run(&d, &["note", "utf8 probe second memory"]);
    run(&d, &["nap", "0-1", "both utf8 probes"]);
    let tree2 = d.join("TREE/2");
    let mut rec = fs::read(&tree2).unwrap();
    rec[..16].copy_from_slice(b"\xff\xfe corrupt bytes");
    fs::write(&tree2, &rec).unwrap();
    let r = run(&d, &["zoom", "0-3"]);
    assert!(
        r.code == 1 && r.stderr.contains("forget 0-1"),
        "{}{}",
        r.stdout,
        r.stderr
    );
}
