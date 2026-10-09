use std::{
    fs,
    io::{self, Write},
    os::unix::process::CommandExt,
    path::{Path, PathBuf},
    process::{self, ExitCode, Stdio},
    sync::Arc,
    time::Instant,
};

use anyhow::{Context, Result, bail};
use clap::{Parser, Subcommand};
use tracing::{error, info, info_span};

use crate::{
    backend::Backend,
    claude::{self, ClaudeCode},
    compact::{Compactor, GaveUp, Options},
    config::Knob,
    cover::cover,
    grep::{Grep, grep},
    hook::{REMINDED, hook},
    log,
    meta::Meta,
    prov,
    record::{Kind, Message, Place, Who, midnight},
    stats::{Stats, stats},
    store::{AtPath, ME, Snapshot, Store, pretty},
    tree::Coord,
    usage::{self, Tally, Use},
    view::{Mem, PLACEHOLDER, VIEW},
    zoom::{page, zoom},
};

const TEMPLATE: &str = "\
## Memory

Your memory:
- The tool is `{memo}`
- Your memories are in `{data}`

This memory outlives every session, compaction, model and vendor change.
Without it you do not know who you are, or what was decided and tried.

### At startup: activating memory (mandatory)

Run `{memo} wake` before any other tool call, in every session, and
then do exactly what it prints, to the end of its output.

### While working: register memories (mandatory)

Call `{memo} note \"<1 line, max {chars} bytes>\"` whenever you learn
something new, or something worth keeping happens. That covers a task
worth real effort, a fact or insight the user teaches you, anything you
learn about their life (even indirectly), any event of lasting effect.

Write only what happened or what you learned: the tool records the time,
machine, repo, branch, agent, model and session of every note, so never
put a date, repo or session in it.

Do not register redundant memories.

Never edit or delete anything under `{data}`: the tool manages it.

### Reading it: the view, zoom, grep

`wake` prints the view: the whole memory in a fixed number of one-line
summaries, oldest first. Each line is `id+n|dates|text`: the n messages
from id on, and the UTC days they span (`2026-10-04..10-05`). A short
message is its own line, word for word; the older the messages, the
more a line covers. Notes (what sessions like this one noted) are bare
text; a chat with the user is tagged by speaker, `user:` and `ai:`.

`{memo} zoom <id+n>` opens a line into the two lines of n/2 it was made
from; `{memo} zoom <id>+1` gives message id in full. Zoom whenever a line
only mentions something you need, before you act, guess or ask.
`{memo} grep <regex>` searches every message, word for word; `-t` adds
the summaries, `--help` lists the filters. `wake`, `zoom` and `grep`
take `--time` for the time of day and `-o repo,branch,...` for more
fields. `{memo} show <id+n>` prints where a message or summary came
from.

### If you're a subagent: skip everything above

Parallel sessions on this machine are all you, and may all write memories.
A subagent is not: it must never run `{memo}`, because it cannot judge what
is already known, and its notes would arrive duplicated and incorrectly.
When you spawn one, write: `You are a subagent. Don't run {memo}.`";

#[derive(Parser, Debug)]
#[command(
    name = "memory",
    about = "A permanent, append-only memory for AI agents.",
    after_help = "The memories live in ~/.ai/memory, or in $AI_MEMORY_DIR if set."
)]
pub struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand, Debug)]
enum Command {
    #[command(about = "create this memory; print the setup block.")]
    Init,
    #[command(about = "read your memory: print the view. Run first, every session.")]
    Wake {
        part: Option<u64>,
        #[arg(help = "the commit a multi-part read is pinned to, as printed")]
        commit: Option<String>,
        #[command(flatten)]
        meta: Meta,
    },
    #[command(about = "record one memory: one short line.")]
    Note {
        #[arg(allow_hyphen_values = true)]
        text: String,
    },
    #[command(about = "build every summary that can be built now, with Claude Code.")]
    Nap,
    #[command(about = "search every message ever recorded, newest page first.")]
    Grep(Grep),
    #[command(about = "open a line of the view into the two lines under it.")]
    Zoom {
        #[arg(help = "a line's id+n, as printed: 2184+8, or 7+1 for message 7")]
        id: String,
        #[arg(help = "the part of a long message to show, as printed")]
        part: Option<u64>,
        #[command(flatten)]
        meta: Meta,
    },
    #[command(about = "show where one message or summary came from.")]
    Show {
        #[arg(help = "a line's id+n, as printed: 2184+8, or 7+1 for message 7")]
        id: String,
    },
    #[command(about = "show this memory's sizes, or change one.")]
    Config {
        #[arg(value_name = "NAME=VALUE")]
        sets: Vec<String>,
    },
    #[command(about = "bulk-load dated notes (bootstrap only).")]
    Import { file: PathBuf },
    #[command(about = "count how often agents wake, note, zoom, grep and show.")]
    Stats(Stats),
    #[command(about = "the prompt hook for agent harnesses: a memory reminder on long prompts.")]
    Hook,
}

impl Command {
    /// Its name and arguments, as typed; a note's text is in the store.
    fn typed(&self) -> (&'static str, Vec<String>) {
        let part = |p: &Option<u64>| p.map(|p| p.to_string());
        match self {
            Command::Init => ("init", Vec::new()),
            Command::Wake {
                part: p,
                commit,
                meta,
            } => (
                "wake",
                part(p)
                    .into_iter()
                    .chain(commit.clone())
                    .chain(meta.args())
                    .collect(),
            ),
            Command::Note { .. } => ("note", Vec::new()),
            Command::Nap => ("nap", Vec::new()),
            Command::Grep(g) => ("grep", g.typed()),
            Command::Zoom { id, part: p, meta } => (
                "zoom",
                std::iter::once(id.clone())
                    .chain(part(p))
                    .chain(meta.args())
                    .collect(),
            ),
            Command::Show { id } => ("show", vec![id.clone()]),
            Command::Config { sets } => ("config", sets.clone()),
            Command::Import { file } => ("import", vec![file.display().to_string()]),
            Command::Stats(st) => ("stats", st.args()),
            Command::Hook => ("hook", Vec::new()),
        }
    }
}

/// How commands reach the compactor's model and stdin; tests swap in their own.
pub struct Runtime {
    pub backend: Box<dyn Fn() -> Result<Arc<dyn Backend>>>,
    pub opts: Options,
    /// Whether `note` starts a background `nap`.
    pub nap_on_note: bool,
    /// Called by `nap` when a round is done, while it still holds the compactor lock.
    pub before_release: Box<dyn Fn()>,
    pub stdin: Box<dyn Fn() -> io::Result<String>>,
}

impl Default for Runtime {
    fn default() -> Runtime {
        Runtime {
            backend: Box::new(|| Ok(Arc::new(ClaudeCode::compactor()?))),
            opts: Options::default(),
            nap_on_note: std::env::var_os("AI_MEMORY_NAP").is_none_or(|v| v != "0"),
            before_release: Box::new(|| {}),
            stdin: Box::new(|| io::read_to_string(io::stdin())),
        }
    }
}

/// Starts `ai memory nap` on `dir`, detached; it logs to `nap.log` there, its stderr too.
fn spawn_nap(dir: &Path) -> Result<()> {
    let exe = std::env::current_exe().context("Cannot find this program to start a nap.")?;
    process::Command::new(exe)
        .args(["memory", "nap"])
        .env("AI_MEMORY_DIR", dir)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(log::open(dir)?)
        .process_group(0)
        .spawn()
        .context("Cannot start a nap.")?;
    Ok(())
}

/// `$AI_MEMORY_DIR`, else `~/.ai/memory`.
pub fn default_dir() -> Result<PathBuf> {
    if let Some(d) = std::env::var_os("AI_MEMORY_DIR").filter(|d| !d.is_empty()) {
        return Ok(d.into());
    }
    let home =
        std::env::home_dir().context("No home directory. Point AI_MEMORY_DIR at a memory.")?;
    Ok(home.join(".ai/memory"))
}

impl Cli {
    /// Runs against the default memory, printing to stdout.
    pub fn exec(self) -> Result<ExitCode> {
        match self.run(&default_dir()?, &mut io::stdout().lock()) {
            Err(e) if broken_pipe(&e) => Ok(ExitCode::SUCCESS),
            r => r,
        }
    }

    pub fn run(self, dir: &Path, out: &mut dyn Write) -> Result<ExitCode> {
        self.run_with(dir, out, &Runtime::default())
    }

    pub fn run_with(self, dir: &Path, out: &mut dyn Write, rt: &Runtime) -> Result<ExitCode> {
        if let Command::Init = self.command {
            return init(dir, out);
        }
        let s = Store::open(dir)?;
        let place = std::env::current_dir()
            .map(|cwd| prov::place(&cwd))
            .unwrap_or_default();
        let who = prov::env_who();
        let (cmd, mut args) = self.command.typed();
        let mut out = Tally::new(out);
        let quiet = matches!(self.command, Command::Hook);
        let r = match self.command {
            Command::Hook => hook(&mut out, (rt.stdin)()).map(|shown| {
                if shown {
                    args.push(REMINDED.into());
                }
                ExitCode::SUCCESS
            }),
            c => c.run(&s, &mut out, rt, &place, &who),
        };
        let error = match &r {
            Ok(c) if *c == ExitCode::SUCCESS => None,
            Ok(_) => Some("exit 1".into()),
            Err(e) => Some(format!("{e:#}")),
        };
        let _ = usage::record(dir, &Use::new(cmd, args, &place, &who, &out, error));
        if quiet { Ok(ExitCode::SUCCESS) } else { r }
    }
}

impl Command {
    fn run(
        self,
        s: &Store,
        out: &mut dyn Write,
        rt: &Runtime,
        place: &Place,
        who: &Who,
    ) -> Result<ExitCode> {
        match self {
            Command::Init | Command::Hook => unreachable!(),
            Command::Wake { part, commit, meta } => {
                wake(s, out, part.unwrap_or(1), commit.as_deref(), &meta)
            }
            Command::Note { text } => note(s, out, &text, rt, place, who),
            Command::Nap => nap(s, out, rt),
            Command::Grep(g) => grep(&s.snapshot()?, out, &g),
            Command::Zoom { id, part, meta } => {
                zoom_cmd(&s.snapshot()?, out, &id, part.unwrap_or(1), &meta)
            }
            Command::Show { id } => show(&s.snapshot()?, out, &id),
            Command::Config { sets } => config(s, out, &sets),
            Command::Import { file } => import(s, out, &file),
            Command::Stats(st) => stats(&s.snapshot()?, s.dir(), out, &st),
        }
    }
}

fn broken_pipe(e: &anyhow::Error) -> bool {
    e.chain()
        .filter_map(|c| c.downcast_ref::<io::Error>())
        .any(|e| e.kind() == io::ErrorKind::BrokenPipe)
}

pub fn plural(n: u64, word: &str) -> String {
    if n == 1 {
        return format!("1 {word}");
    }
    let word = match word.strip_suffix('y') {
        Some(stem) => format!("{stem}ie"),
        None if word.ends_with(['s', 'h', 'x']) => format!("{word}e"),
        None => word.to_string(),
    };
    format!("{n} {word}s")
}

/// A printed `id+n` (a bare `id` is `id+1`) that names a line within `t` messages.
pub fn line_id(s: &str, t: u64) -> Result<Coord> {
    let Some((id, n)) = Coord::parse(s) else {
        bail!("'{s}' is not an id+n. Copy one from the view, like 2184+8.");
    };
    Coord::at(id, n, t).with_context(|| format!("No line {id}+{n}."))
}

fn check(text: &str, limit: u64) -> Result<&str> {
    let text = text.trim();
    if text.is_empty() {
        bail!("Empty. A memory is one line of text.");
    }
    if text.contains(['\n', '\r']) {
        bail!(
            "{} lines. A memory is one line: merge them, or note them separately.",
            text.matches('\n').count() + 1
        );
    }
    let n = text.len() as u64;
    if n > limit {
        bail!(
            "Too long: {n} bytes, limit {limit}. Accented characters cost 2 bytes. Compress it further."
        );
    }
    Ok(text)
}

fn paginate(lines: Vec<String>, max_lines: u64, max_chars: u64) -> Vec<Vec<String>> {
    let (mut parts, mut cur, mut size) = (Vec::new(), Vec::new(), 0);
    for line in lines {
        let n = line.len() as u64 + 1;
        if !cur.is_empty() && (cur.len() as u64 >= max_lines || size + n > max_chars) {
            parts.push(std::mem::take(&mut cur));
            size = 0;
        }
        cur.push(line);
        size += n;
    }
    if !cur.is_empty() {
        parts.push(cur);
    }
    parts
}

fn init(dir: &Path, out: &mut dyn Write) -> Result<ExitCode> {
    let (s, fresh) = Store::create(dir)?;
    let at = pretty(dir);
    if fresh {
        writeln!(out, "Created {at}: your memory.")?;
    } else {
        let n = s.snapshot()?.log_len()?;
        writeln!(out, "Found {at}: {}.", plural(n, "message"))?;
    }
    writeln!(
        out,
        "Sizes live in {at}/config as ai.memory.*; the defaults are fine.\n"
    )?;
    writeln!(
        out,
        "Paste this at the top of your agent's AGENTS.md (or CLAUDE.md), done:\n"
    )?;
    let block = TEMPLATE
        .replace("{memo}", ME)
        .replace("{data}", &at)
        .replace("{chars}", &s.cfg.get(Knob::EntryChars).to_string());
    writeln!(out, "{block}")?;
    Ok(ExitCode::SUCCESS)
}

fn wake(
    s: &Store,
    out: &mut dyn Write,
    k: u64,
    commit: Option<&str>,
    meta: &Meta,
) -> Result<ExitCode> {
    let snap = match commit {
        Some(c) => s.at(Some(s.commit_id(c)?))?,
        None => s.snapshot()?,
    };
    let t = snap.log_len()?;
    let Some(pin) = snap.commit().filter(|_| t > 0) else {
        writeln!(
            out,
            "No memories yet. Record the first with: {ME} note \"<one line>\""
        )?;
        writeln!(out, "You are awake.")?;
        return Ok(ExitCode::SUCCESS);
    };
    let mut lines = Vec::new();
    for c in cover(t, s.cfg.get(Knob::WakeLines)) {
        wake_lines(&snap, c, meta, &mut lines)?;
    }
    let parts = paginate(
        lines,
        s.cfg.get(Knob::PartLines),
        s.cfg.get(Knob::PartChars),
    );
    let n = parts.len() as u64;
    if !(1..=n).contains(&k) {
        bail!(
            "No part {k}: the memory has {}. Run: {ME} wake",
            plural(n, "part")
        );
    }
    if n > 1 {
        writeln!(
            out,
            "Your memory, part {k} of {n}, oldest first ({}).",
            plural(t, "message")
        )?;
    }
    writeln!(out, "{}", parts[k as usize - 1].join("\n"))?;
    if k < n {
        writeln!(
            out,
            "Not awake yet. Run: {ME} wake {} {pin}{}",
            k + 1,
            meta.flags()
        )?;
    } else {
        writeln!(out, "You are awake.")?;
    }
    Ok(ExitCode::SUCCESS)
}

/// Line `c`, or its halves' lines while its summary is not built; tool calls have none.
fn wake_lines(snap: &Snapshot, c: Coord, meta: &Meta, out: &mut Vec<String>) -> Result<()> {
    let text = match snap.node(c)? {
        Some(n) if n.text.is_empty() => return Ok(()),
        Some(n) => n.text,
        None if c.l == 0 => PLACEHOLDER.into(),
        None => {
            for k in c.children() {
                wake_lines(snap, k, meta, out)?;
            }
            return Ok(());
        }
    };
    out.push(format!("{c}|{}{text}", meta.head_at(snap, c)?));
    Ok(())
}

fn note(
    s: &Store,
    out: &mut dyn Write,
    text: &str,
    rt: &Runtime,
    place: &Place,
    who: &Who,
) -> Result<ExitCode> {
    let text = check(text, s.cfg.get(Knob::EntryChars))?;
    let m = Message {
        place: place.clone(),
        who: who.clone(),
        ..Message::new(Kind::Note, text)
    };
    let (i, _, _) = s.append("note", &[m])?;
    writeln!(out, "Saved as {i}+1.")?;
    if rt.nap_on_note && pending(s)? {
        spawn_nap(s.dir())?;
    }
    Ok(ExitCode::SUCCESS)
}

/// The compactor's lock is taken: another nap or chat is building, and will see the new messages.
const BUSY: &str = "A compactor is already running; it will build these too.";

/// Whether a node can be built now on the latest snapshot.
fn pending(s: &Store) -> Result<bool> {
    let mem = Mem::load(&s.snapshot()?, VIEW)?;
    Ok(!mem.due(|_| false, 1).is_empty())
}

fn nap(s: &Store, out: &mut dyn Write, rt: &Runtime) -> Result<ExitCode> {
    let _log = log::init(s.dir())?;
    let _nap = info_span!(
        "nap",
        pid = process::id(),
        model = %claude::model(),
        dir = ?pretty(s.dir())
    )
    .entered();
    let start = Instant::now();
    info!("start");
    let e = match rounds(s, out, rt) {
        Ok(built) => {
            info!(built, secs = log::secs(start.elapsed()), "end");
            return Ok(ExitCode::SUCCESS);
        }
        Err(e) => e,
    };
    let Some(g) = e.downcast_ref::<GaveUp>() else {
        error!(error = format!("{e:#}"), "failed");
        return Err(e);
    };
    error!(node = %g.node, tries = g.tries, error = g.error, "gave up");
    writeln!(
        out,
        "{g}. It stays pending; the nap after the next note tries again."
    )?;
    Ok(ExitCode::FAILURE)
}

/// Compactor rounds until nothing is due or another compactor holds the lock; the nodes built.
fn rounds(s: &Store, out: &mut dyn Write, rt: &Runtime) -> Result<u64> {
    let mut built = 0;
    while pending(s)? {
        let Some(mut c) = Compactor::new(s, (rt.backend)()?, rt.opts)? else {
            writeln!(out, "{BUSY}")?;
            info!("busy: another compactor holds the lock; exiting");
            return Ok(built);
        };
        let retries = rt.opts.retries;
        c.run(|c| {
            for r in c.take_reports() {
                writeln!(
                    out,
                    "Failed {r}. Retrying up to {retries} times, waiting longer each time."
                )?;
            }
            Ok(())
        })?;
        built += c.built();
        (rt.before_release)();
        drop(c);
    }
    match built {
        0 => writeln!(out, "Nothing to build.")?,
        n => writeln!(out, "Built {}.", plural(n, "summary"))?,
    }
    Ok(built)
}

fn config(s: &Store, out: &mut dyn Write, sets: &[String]) -> Result<ExitCode> {
    let mut cfg = s.cfg.clone();
    for a in sets {
        let knob = a
            .split_once('=')
            .and_then(|(k, v)| Some((Knob::parse(&k.trim().to_uppercase())?, v.trim())));
        let Some((k, v)) = knob else {
            bail!(
                "usage: {ME} config [NAME=VALUE ...]   # NAME one of {}",
                Knob::names()
            );
        };
        cfg.set(
            k,
            if v.is_empty() {
                None
            } else {
                Some(k.validate(v, k.name())?)
            },
        );
    }
    if !sets.is_empty() {
        s.save_config(&cfg)?;
    }
    for k in Knob::ALL {
        let note = match cfg.overridden(k) {
            Some(_) => format!(" (default {})", k.default()),
            None => String::new(),
        };
        writeln!(out, "{:<12} {:<7} {}{note}", k.name(), cfg.get(k), k.what())?;
    }
    Ok(ExitCode::SUCCESS)
}

fn zoom_cmd(
    s: &Snapshot,
    out: &mut dyn Write,
    id: &str,
    part: u64,
    meta: &Meta,
) -> Result<ExitCode> {
    let c = line_id(id, s.log_len()?)?;
    let Some(text) = zoom(s, c.id(), c.n(), Some(meta))? else {
        bail!("No line {c}.");
    };
    let (text, n) = page(&text, s.cfg().get(Knob::PartChars), part)?;
    writeln!(out, "{}", text.strip_suffix('\n').unwrap_or(text))?;
    if part < n {
        writeln!(
            out,
            "Part {part} of {n}. Next: {ME} zoom {c} {}{}",
            part + 1,
            meta.flags()
        )?;
    }
    Ok(ExitCode::SUCCESS)
}

fn show(s: &Snapshot, out: &mut dyn Write, id: &str) -> Result<ExitCode> {
    let c = line_id(id, s.log_len()?)?;
    let fields: Vec<(&str, String)> = if c.l == 0 {
        let m = s.message(c.i)?;
        let (p, w) = (m.place, m.who);
        vec![
            ("ts", m.ts),
            ("kind", m.kind.name().into()),
            ("origin", m.origin),
            ("repo", p.repo),
            ("head", p.head),
            ("branch", p.branch),
            ("agent", w.agent),
            ("model", w.model),
            ("session", w.session),
            ("text", m.text),
        ]
    } else {
        let Some(n) = s.node(c)? else {
            bail!("{c} is not summarized yet. Run: {ME} zoom {c}");
        };
        vec![
            ("ts", n.ts),
            ("origin", n.origin),
            ("agent", n.who.agent),
            ("model", n.who.model),
            ("session", n.who.session),
            ("text", n.text),
        ]
    };
    writeln!(out, "{c}")?;
    for (k, v) in fields {
        writeln!(out, "{k:<8}{v}")?;
    }
    Ok(ExitCode::SUCCESS)
}

fn is_iso_date(d: &str) -> bool {
    d.len() == 10
        && d.bytes().enumerate().all(|(i, b)| {
            if i == 4 || i == 7 {
                b == b'-'
            } else {
                b.is_ascii_digit()
            }
        })
}

fn import(s: &Store, out: &mut dyn Write, file: &Path) -> Result<ExitCode> {
    let Ok(src) = String::from_utf8(fs::read(file).at(file)?) else {
        bail!(
            "{} is not UTF-8 text. Convert it, then import again.",
            pretty(file)
        );
    };
    let mut last = String::new();
    let mut items = Vec::new();
    for (i, line) in (1..).zip(src.lines()) {
        if line.trim().is_empty() {
            continue;
        }
        let (date, text) = line.split_once(' ').unwrap_or((line, ""));
        if !is_iso_date(date) {
            bail!("line {i}: expected 'YYYY-MM-DD <text>', got: {line}");
        }
        let Ok(day) = date.parse::<jiff::civil::Date>() else {
            bail!("line {i}: {date} is not a real date.");
        };
        if *date < *last {
            bail!("line {i}: date {date} precedes the previous line's ({last}).");
        }
        let text = text.trim();
        if text.is_empty() {
            bail!("line {i}: no text.");
        }
        last = date.to_string();
        items.push(Message {
            ts: midnight(day),
            ..Message::new(Kind::Note, text)
        });
    }
    if items.is_empty() {
        bail!("{} has no notes.", file.display());
    }
    let (base, _, _) = s.append("import", &items)?;
    let k = items.len() as u64;
    writeln!(
        out,
        "Imported {}, {base}+1 to {}+1.",
        plural(k, "note"),
        base + k - 1
    )?;
    Ok(ExitCode::SUCCESS)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plurals() {
        assert_eq!(plural(1, "message"), "1 message");
        assert_eq!(plural(2, "memory"), "2 memories");
        assert_eq!(plural(5, "match"), "5 matches");
        assert_eq!(plural(0, "part"), "0 parts");
    }

    #[test]
    fn line_ids() {
        assert_eq!(line_id("16+16", 32).unwrap(), Coord::new(4, 1));
        assert_eq!(line_id("7", 8).unwrap(), Coord::leaf(7));
        assert_eq!(line_id("7+1", 8).unwrap(), Coord::leaf(7));
        for bad in ["16+16", "3+2", "4+3", "x", "#7", "7-8", "31+1"] {
            assert!(line_id(bad, 31).is_err(), "{bad}");
        }
    }

    #[test]
    fn pages_respect_both_caps() {
        let lines: Vec<_> = (0..10).map(|i| format!("line {i}")).collect();
        assert_eq!(paginate(lines.clone(), 3, 1000).len(), 4);
        assert_eq!(paginate(lines, 100, 14).len(), 10 / 2);
    }
}
