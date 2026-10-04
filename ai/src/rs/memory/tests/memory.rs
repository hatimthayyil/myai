use std::{
    fs,
    path::{Path, PathBuf},
    process::ExitCode,
    sync::LazyLock,
    thread,
};

use ai_memory::{
    Changes, Cli, Knob, LOG, REC, Store, Summary, level, pending, pending_count, seg_path,
};
use clap::Parser;
use regex::Regex;
use sha2::Digest;
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
    let result = cli.run(dir, &mut buf);
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
    (tmp, d)
}

fn log_len(d: &Path) -> u64 {
    Store::open(d)
        .unwrap()
        .snapshot()
        .unwrap()
        .log_len()
        .unwrap()
}

fn built(d: &Path) -> u64 {
    let s = Store::open(d).unwrap();
    let snap = s.snapshot().unwrap();
    let t = snap.log_len().unwrap();
    (1..64)
        .map(|k| 1 << k)
        .take_while(|&size| size <= t)
        .map(|size| snap.level_len(size).unwrap())
        .sum()
}

fn log_bytes(d: &Path) -> Vec<u8> {
    let s = Store::open(d).unwrap();
    let snap = s.snapshot().unwrap();
    (0..8)
        .flat_map(|k| snap.read(&seg_path(LOG, k)).unwrap().to_vec())
        .collect()
}

fn state(d: &Path) -> (Option<String>, Vec<u8>) {
    let s = Store::open(d).unwrap();
    let head = s.snapshot().unwrap().commit().map(|c| c.to_string());
    (head, fs::read(d.join("config")).unwrap())
}

fn rewrite(d: &Path, path: &str, edit: impl Fn(&mut Vec<u8>)) {
    let s = Store::open(d).unwrap();
    s.mutate("test", |snap| {
        let mut b = snap.read(path)?.to_vec();
        edit(&mut b);
        let mut ch = Changes::default();
        ch.put(path.into(), b);
        Ok(((), ch))
    })
    .unwrap();
}

fn ids(d: &Path, args: &[&str]) -> Vec<(u64, u64)> {
    let r = run(d, &[&["zoom"], args].concat());
    assert_eq!(r.code, 0, "zoom {args:?} failed: {}", r.stderr);
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
        !fs::read_to_string(d.join("config"))
            .unwrap()
            .contains("wakeLines"),
        "a store wrote its own sizes"
    );
    let s = Store::open(d).unwrap();
    let snap = s.snapshot().unwrap();
    let ts: Vec<_> = (0..3).map(|i| snap.log_get(i).unwrap().ts).collect();
    assert_eq!(
        ts,
        ["20200101T000000Z", "20200101T000001Z", "20200101T000002Z"]
    );
    assert_eq!(snap.log_get(5).unwrap().ts, "20200102T000000Z");
    let keys: Vec<_> = (0..N).map(|i| snap.log_get(i).unwrap().key()).collect();
    assert!(keys.windows(2).all(|w| w[0] < w[1]), "keys out of order");

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

    run(d, &["note", "one more thing happened today"]);
    assert_eq!(log_len(d), N + 1, "note did not append");
    let s = Store::open(d).unwrap();
    let snap = s.snapshot().unwrap();
    for dir in [LOG.to_string(), level(2), level(1024)] {
        for k in 0..8 {
            assert_eq!(snap.read(&seg_path(&dir, k)).unwrap().len() % REC, 0);
        }
    }
    assert_eq!(
        snap.read(&seg_path(LOG, 7)).unwrap().len(),
        (N + 1 - 7 * 256) as usize * REC
    );

    let r = run(d, &["nap", "0-1", "attempted overwrite"]);
    assert!(r.code == 0 && r.stdout.contains("Nothing left to compress"));

    let r = run(d, &["grep", "memory number 7,"]);
    let origin = Store::open(d).unwrap().origin().unwrap();
    assert_eq!(
        r.stdout,
        format!(
            "#7 2020-01-02 00:00 {origin} - {}\n1 match.\n",
            snap.log_get(7).unwrap().text
        )
    );
    assert!(
        run(d, &["grep", "^memory number 7,"])
            .stdout
            .contains("#7 ")
    );
    for (args, hit) in [
        (&["2020-01-02"][..], false),
        (&["Memory Number 7,"], false),
        (&["-i", "Memory Number 7,"], true),
        (&["-s", "-i", "Memory Number 7,"], true),
        (&["-i", "-s", "Memory Number 7,"], false),
        (&["number 7,", "--since", "2020-01-02"], true),
        (&["number 7,", "--since", "2020-01-03"], false),
        (&["-F", "number 7, a"], true),
        (&["-F", "number 7.*"], false),
        (&["-F", "("], false),
    ] {
        let r = run(d, &[&["grep"], args].concat());
        assert_eq!(r.code, 0, "grep {args:?}: {}", r.stderr);
        assert_eq!(
            r.stdout != "No match.\n",
            hit,
            "grep {args:?}: {}",
            r.stdout
        );
    }
    let r = run(d, &["grep", "("]);
    assert!(
        r.code == 1 && r.stderr.contains("bad regex"),
        "{}",
        r.stderr
    );
    assert_eq!(
        run(d, &["grep", "memory number", "-c"]).stdout,
        "2000 matches.\n"
    );

    let r = run(d, &["grep", "memory number", "-m", "5"]);
    let got: Vec<_> = r
        .stdout
        .lines()
        .map(|l| l.split(' ').next().unwrap())
        .collect();
    assert_eq!(got, ["#1995", "#1996", "#1997", "#1998", "#1999", "Newest"]);
    assert!(
        r.stdout.ends_with(
            "Newest 5 of 2000. Older: ai memory grep 'memory number' -m 5 --before 1995\n"
        ),
        "{}",
        r.stdout
    );
    let (mut seen, mut cmd) = (Vec::new(), vec!["grep".to_string(), "memory number".into()]);
    loop {
        let r = run(d, &cmd.iter().map(String::as_str).collect::<Vec<_>>());
        assert!(r.code == 0 && r.stdout.len() < CAP_CHARS, "{}", r.stderr);
        let mut page: Vec<u64> = r
            .stdout
            .lines()
            .filter_map(|l| l.strip_prefix('#')?.split(' ').next()?.parse().ok())
            .collect();
        page.append(&mut seen);
        seen = page;
        let Some(next) = r.stdout.lines().last().unwrap().split(" --before ").nth(1) else {
            break;
        };
        cmd = vec![
            "grep".into(),
            "memory number".into(),
            "--before".into(),
            next.into(),
        ];
    }
    assert_eq!(
        seen,
        (0..N).collect::<Vec<_>>(),
        "paging lost or repeated a match"
    );

    let pat = "memory number 7[0-9],";
    let lines = |out: &str| -> Vec<String> {
        out.lines()
            .filter(|l| l.starts_with('#'))
            .map(String::from)
            .collect()
    };
    let all = run(d, &["grep", pat, "-t"]).stdout;
    assert!(
        all.contains("#70 ") && all.contains("#70-71 ") && all.contains("#72-79 "),
        "{all}"
    );
    let (mut paged, mut before) = (Vec::new(), None::<String>);
    loop {
        let mut args = vec!["grep", pat, "-t", "-m", "3"];
        if let Some(b) = &before {
            args.extend(["--before", b]);
        }
        let out = run(d, &args).stdout;
        paged.splice(0..0, lines(&out));
        match out.lines().last().unwrap().split(" --before ").nth(1) {
            Some(b) => before = Some(b.into()),
            None => break,
        }
    }
    assert_eq!(paged, lines(&all), "tree paging differs from one page");

    let target = 777;
    let (mut b, mut calls) = ((0, 1024), 0);
    loop {
        let kids = ids(d, &[&format!("{}-{}", b.0, b.1 - 1)]);
        calls += 1;
        assert_eq!((kids[0].0, kids[kids.len() - 1].1), b, "zoom left a gap");
        assert!(kids.windows(2).all(|w| w[0].1 == w[1].0));
        if kids.iter().all(|k| k.1 - k.0 == 1) {
            assert_eq!(kids.len(), 16);
            break;
        }
        assert_eq!(kids.len(), 8, "depth 3 is 8 nodes");
        for k in &kids {
            ids(d, &[&format!("{}-{}", k.0, k.1 - 1)]);
        }
        b = *kids.iter().find(|k| k.0 <= target && target < k.1).unwrap();
    }
    assert_eq!(calls, 3, "1024 -> 128 -> 16 -> raw");
    assert_eq!(ids(d, &["0-1023", "--depth", "1"]), [(0, 512), (512, 1024)]);
    assert_eq!(ids(d, &["768-783", "--depth", "1"]).len(), 16);
    assert_eq!(ids(d, &["0-31", "--depth", "4"]).len(), 32);
    assert_eq!(ids(d, &["#777"]), [(777, 778)]);
    assert_eq!(ids(d, &["777"]), ids(d, &["776-777"])[1..]);
    assert!(
        run(d, &["zoom", "776-777"])
            .stdout
            .contains(&format!("memory number {target},"))
    );
    let r = run(d, &["zoom", "0-127", "--depth", "6"]);
    assert!(
        r.code == 1
            && r.stderr.contains("128 lines")
            && r.stderr.contains("use --depth 5")
            && r.stderr.contains("Run: ai memory zoom 0-127 --depth 5"),
        "{}",
        r.stderr
    );
    assert_eq!(ids(d, &["0-127", "--depth", "5"]).len(), 32);
    run(d, &["config", "PART_CHARS=1000"]);
    let r = run(d, &["zoom", "0-1023"]);
    assert!(
        r.code == 1 && r.stderr.contains("Run: ai memory zoom 0-1023 --depth 1"),
        "{}",
        r.stderr
    );
    assert_eq!(
        ids(d, &["768-783"]).len(),
        16,
        "a raw block is never refused"
    );
    run(d, &["config", "PART_CHARS="]);
    for bad in [&["0-1", "--depth", "0"][..], &["0-1", "--depth", "7"]] {
        assert_eq!(run(d, &[&["zoom"], bad].concat()).code, 2, "{bad:?}");
    }

    let r = run(d, &["zoom", "1024-2047"]);
    assert!(
        r.stdout.contains("#1920-2047 not compressed yet"),
        "{}",
        r.stdout
    );
    let r = run(d, &["zoom", "1024-2047", "--depth", "1"]);
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

    let (before, log) = (built(d), log_bytes(d));
    let r = run(d, &["forget", "16-31"]);
    assert!(r.stdout.contains("16-31"), "{}{}", r.stdout, r.stderr);
    assert!(built(d) < before, "forget did not shrink the tree");
    assert_eq!(log_bytes(d), log, "forget touched the log");
    assert_eq!(
        run(d, &["wake"]).code,
        1,
        "wake should refuse after a forget"
    );
    let mid = state(d);
    let r = run(d, &["nap", "0-1", "attempted overwrite"]);
    assert!(
        r.code == 0 && r.stdout.contains("already settled"),
        "{}",
        r.stderr
    );
    assert_eq!(
        state(d),
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
    assert_eq!(built(d), before, "tree did not return to its original size");
    assert_eq!(run(d, &["forget", "17-32"]).code, 1);
    run(d, &["forget", "16-31"]);
    let z = run(d, &["zoom", "0-31", "--depth", "1"]).stdout;
    assert!(z.contains("#16-31 not compressed yet"), "{z}");
    settle(d, "rebuilt after forget");
    assert_eq!(built(d), before);
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
    assert!(run(d, &["grep", "coração"]).stdout.contains("João"));
    assert!(run(d, &["grep", "CORAÇÃO", "-i"]).stdout.contains("João"));
    assert!(
        run(d, &["grep", "plain ascii memory right after"])
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

    let t0 = log_len(d);
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
    let snap = s.snapshot().unwrap();
    for t in (1..40).chain([t0 - 1, t0, t0 + 1]) {
        assert_eq!(
            pending_count(&snap, t).unwrap(),
            pending(&snap, t, None).unwrap().len() as u64,
            "pending_count disagrees with pending at T={t}"
        );
    }

    let r = run(d, &["grep", "memory number"]);
    assert!(r.stdout.len() < CAP_CHARS);
    assert!(
        r.stdout
            .contains("Older: ai memory grep 'memory number' --before ")
    );

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

    run(d, &["config", "WAKE_LINES=12"]);
    let before = state(d);
    assert!(before.0.is_some() && String::from_utf8_lossy(&before.1).contains("wakeLines = 12"));
    for _ in 0..3 {
        let r = run(d, &["init"]);
        assert!(r.code == 0 && r.stdout.contains("Found"));
    }
    assert_eq!(state(d), before, "init modified an existing memory");
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
    assert!(r.stdout.contains("your memory."));
    assert!(
        r.stdout
            .contains("Parallel sessions on this machine are all you")
    );
    assert!(!r.stdout.contains("OptMem") && !r.stdout.contains("repository"));
    assert!(d.join("HEAD").exists() && d.join("objects").is_dir());
    assert!(run(&d, &["init"]).stdout.contains("Found"));
    assert!(run(&d, &["wake"]).stdout.contains("You are awake."));
}

#[test]
fn init_refuses_a_foreign_dir() {
    let tmp = TempDir::new().unwrap();
    fs::write(tmp.path().join("LOG.txt"), "#0 2026-01-01 an old store\n").unwrap();
    let r = run(tmp.path(), &["init"]);
    assert!(
        r.code == 1 && r.stderr.contains("not a memory"),
        "{}",
        r.stderr
    );
    assert!(run(tmp.path(), &["wake"]).stderr.contains("No memory at"));
    assert!(!tmp.path().join("HEAD").exists());
}

#[test]
fn a_bad_knob_names_its_key() {
    let (_tmp, d) = store();
    let mut cfg = fs::read_to_string(d.join("config")).unwrap();
    cfg += "[ai \"memory\"]\n\twakeLines = many\n";
    fs::write(d.join("config"), cfg).unwrap();
    for c in ["wake", "config"] {
        let r = run(&d, &[c]);
        assert!(
            r.code == 1
                && r.stderr.contains("config: ai.memory.wakeLines")
                && r.stderr.contains("'many'"),
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
        r.code == 1 && r.stderr.ends_with("file: File exists."),
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
            sc.spawn(move || {
                let r = run(d, &["note", &format!("parallel note {i}")]);
                assert_eq!(r.code, 0, "{}", r.stderr);
            });
        }
    });
    let s = Store::open(&d).unwrap();
    let snap = s.snapshot().unwrap();
    let all = snap.log_slice(0, snap.log_len().unwrap()).unwrap();
    let mut texts: Vec<_> = all.iter().map(|m| m.text.clone()).collect();
    texts.sort();
    let mut want: Vec<_> = (0..p).map(|i| format!("parallel note {i}")).collect();
    want.sort();
    assert_eq!(texts, want);
    assert!(
        all.windows(2).all(|w| w[0].key() < w[1].key()),
        "keys not strictly increasing"
    );
    assert!(
        all.iter()
            .all(|m| m.origin == all[0].origin && m.origin.len() == 6)
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
fn summaries_carry_a_fingerprint_of_their_block() {
    let (_tmp, d) = store();
    for i in 0..2 {
        run(&d, &["note", &format!("fingerprinted memory {i}")]);
    }
    run(&d, &["nap", "0-1", "both"]);
    let s = Store::open(&d).unwrap();
    let snap = s.snapshot().unwrap();
    let rec = snap.read(&seg_path(&level(2), 0)).unwrap();
    let sum = Summary::decode(&rec).unwrap();
    let keys: Vec<_> = snap
        .log_slice(0, 2)
        .unwrap()
        .iter()
        .map(|m| m.key())
        .collect();
    let want = format!("{}\n{}\n", keys[0], keys[1]);
    let digest = sha2::Sha256::digest(want.as_bytes());
    let hex: String = digest[..8].iter().map(|b| format!("{b:02x}")).collect();
    assert_eq!(sum.fp, hex);
    assert_eq!((sum.text.as_str(), sum.origin.len()), ("both", 6));
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
    run(&d, &["config", "WAKE_LINES=2"]);
    rewrite(&d, &seg_path(&level(2), 0), |b| b[..REC - 1].fill(b' '));
    let r = run(&d, &["wake"]);
    assert!(
        r.code == 1 && r.stderr.contains("forget 0-1"),
        "{}{}",
        r.stdout,
        r.stderr
    );

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
    rewrite(&d, &seg_path(&level(16), 0), |b| b[..REC - 1].fill(b' '));
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
    rewrite(&d, &seg_path(&level(2), 0), |b| {
        b[..16].copy_from_slice(b"\xff\xfe corrupt bytes")
    });
    for args in [&["show", "0-1"][..], &["grep", "-t", "probe"]] {
        let r = run(&d, args);
        assert!(
            r.code == 1 && r.stderr.contains("forget 0-1"),
            "{args:?}: {}{}",
            r.stdout,
            r.stderr
        );
    }
}

fn heads(out: &str) -> Vec<&str> {
    out.lines()
        .map(|l| match l.starts_with('#') {
            true => l.split(' ').next().unwrap(),
            false => l,
        })
        .collect()
}

#[test]
fn grep_pages_with_context() {
    let (_tmp, d) = store();
    for i in 0..10 {
        let word = if [2, 3, 7].contains(&i) {
            "hit"
        } else {
            "miss"
        };
        run(&d, &["note", &format!("{word} {i}")]);
    }
    let grep = |args: &[&str]| run(&d, &[&["grep", "hit"], args].concat()).stdout;
    assert_eq!(
        heads(&grep(&["-C", "1"])),
        ["#1", "#2", "#3", "#4", "--", "#6", "#7", "#8", "3 matches."]
    );
    assert_eq!(heads(&grep(&[])), ["#2", "#3", "#7", "3 matches."]);
    assert_eq!(grep(&["-c"]), "3 matches.\n");
    assert_eq!(grep(&["-c", "--before", "7"]), "2 matches.\n");
    let out = grep(&["-m", "1", "-C", "1"]);
    assert_eq!(heads(&out)[..3], ["#6", "#7", "#8"]);
    assert!(
        out.ends_with("Newest 1 of 3. Older: ai memory grep 'hit' -m 1 -C 1 --before 7\n"),
        "{out}"
    );
    assert_eq!(
        heads(&grep(&["-m", "1", "-C", "1", "--before", "7"]))[..2],
        ["#3", "#4"]
    );
    assert_eq!(run(&d, &["grep", "nothing"]).stdout, "No match.\n");
    assert_eq!(run(&d, &["grep", "nothing", "-c"]).stdout, "No match.\n");
    assert_eq!(grep(&["--origin", "NOPE00"]), "No match.\n");
    assert_eq!(grep(&["--repo", "acme/widget"]), "No match.\n");

    run(&d, &["nap", "0-1", "summary with a hit"]);
    run(&d, &["nap", "2-3", "summary without"]);
    assert_eq!(heads(&grep(&["-t"]))[..3], ["#0-1", "#2", "#3"]);
    assert_eq!(heads(&grep(&["-t", "-C", "1"]))[..3], ["#0-1", "--", "#1"]);
    assert_eq!(grep(&["-t", "-c"]), "4 matches.\n");
    let r = run(&d, &["grep", "hit", "--before", "x"]);
    assert!(
        r.code == 1 && r.stderr.contains("not an id"),
        "{}",
        r.stderr
    );
}

#[test]
fn show_prints_every_field() {
    let (_tmp, d) = store();
    for i in 0..3 {
        run(&d, &["note", &format!("shown memory {i}")]);
    }
    let s = Store::open(&d).unwrap();
    let origin = s.origin().unwrap();
    let m = s.snapshot().unwrap().log_get(1).unwrap();
    let out = run(&d, &["show", "#1"]).stdout;
    let fields: Vec<_> = out.lines().map(|l| l.split(' ').next().unwrap()).collect();
    assert_eq!(
        fields,
        [
            "#1", "ts", "origin", "repo", "head", "branch", "agent", "model", "session", "text"
        ]
    );
    assert!(out.contains(&format!("ts      {}\n", m.ts)));
    assert!(out.contains(&format!("origin  {origin}\n")));
    assert!(out.ends_with("text    shown memory 1\n"));

    let r = run(&d, &["show", "0-1"]);
    assert!(
        r.code == 1 && r.stderr.contains("not compressed yet"),
        "{}",
        r.stderr
    );
    run(&d, &["nap", "0-1", "the first two"]);
    let fp = s.snapshot().unwrap().fingerprint(0, 2).unwrap();
    let out = run(&d, &["show", "0-1"]).stdout;
    let fields: Vec<_> = out.lines().map(|l| l.split(' ').next().unwrap()).collect();
    assert_eq!(
        fields,
        [
            "#0-1", "ts", "origin", "fp", "agent", "model", "session", "text"
        ]
    );
    assert!(out.contains(&format!("fp      {fp}\n")) && out.ends_with("text    the first two\n"));
    let r = run(&d, &["show", "3"]);
    assert!(
        r.code == 1 && r.stderr.contains("beyond the memory"),
        "{}",
        r.stderr
    );
    assert_eq!(run(&d, &["show", "1-2"]).code, 1);
}
