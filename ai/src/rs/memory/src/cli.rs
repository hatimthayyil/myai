use std::{
    fs,
    io::{self, Write},
    path::{Path, PathBuf},
    process::ExitCode,
    time::{Duration, Instant},
};

use anyhow::{Context, Result, bail};
use clap::{Parser, Subcommand};
use jiff::ToSpan;

use crate::{
    config::Knob,
    cover::{Block, cover},
    grep::{Grep, grep},
    nap::{RAW_MAX, blank, next_nap, pending, pending_count},
    prov,
    record::{Memory, later, midnight, now},
    store::{AtPath, ME, Put, Snapshot, Store, pretty},
    sync::{Report, remote, sync},
};

const WAKE_SYNC: Duration = Duration::from_secs(3);
const ZOOM_LINES: usize = 64;

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

Do not register redundant memories.

If `{memo} note` asks a compression: do it before your next action.

Never edit or delete anything under `{data}`: the tool manages it.

### When you need an old memory: search, or navigate

`{memo} grep <regex>` searches every memory, word for word; `-t` adds
the summaries, `--help` lists the filters.

Your memories also form a binary tree: #0-1, #2-3 ... exist as one-line
summaries, pairs of those as #0-3, and so on -- every `#a-b` line wake
prints is one node of it. `{memo} zoom <a-b>` opens a node three levels
deep (`--depth 1`-`6`); small nodes open to the raw memories.
`{memo} show <id>` prints where one memory or summary came from.

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
    #[command(about = "read your memory. Run first, every session.")]
    Wake { part: Option<u64>, t: Option<u64> },
    #[command(about = "record one memory: one short line.")]
    Note {
        #[arg(allow_hyphen_values = true)]
        text: String,
    },
    #[command(about = "do the pending compressions.")]
    Nap {
        #[arg(requires = "text")]
        id: Option<String>,
        #[arg(allow_hyphen_values = true)]
        text: Option<String>,
    },
    #[command(about = "search every memory ever recorded, newest page first.")]
    Grep(Grep),
    #[command(about = "open a tree node, a few levels down.")]
    Zoom {
        #[arg(help = "a printed id: 16-31, or one memory's 7")]
        id: String,
        #[arg(
            long,
            default_value_t = 3,
            value_parser = clap::value_parser!(u8).range(1..=6),
            help = "levels to open"
        )]
        depth: u8,
    },
    #[command(about = "show where one memory or summary came from.")]
    Show {
        #[arg(help = "a printed id: 16-31, or one memory's 7")]
        id: String,
    },
    #[command(about = "drop a bad summary; nap rebuilds it.")]
    Forget { id: String },
    #[command(about = "show this memory's sizes, or change one.")]
    Config {
        #[arg(value_name = "NAME=VALUE")]
        sets: Vec<String>,
    },
    #[command(about = "exchange memories with this memory's git remote.")]
    Sync,
    #[command(about = "bulk-load dated memories (bootstrap only).")]
    Import { file: PathBuf },
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
        if let Command::Init = self.command {
            return init(dir, out);
        }
        let s = Store::open(dir)?;
        match self.command {
            Command::Init => unreachable!(),
            Command::Wake { part, t } => {
                if t.is_none() {
                    wake_sync(&s, out)?;
                }
                wake(&s.snapshot()?, out, part.unwrap_or(1), t)
            }
            Command::Note { text } => note(&s, out, &text),
            Command::Nap { id, text } => nap(&s, out, id.as_deref().zip(text.as_deref())),
            Command::Grep(g) => grep(&s.snapshot()?, out, &g),
            Command::Zoom { id, depth } => zoom(&s.snapshot()?, out, &id, depth.into()),
            Command::Show { id } => show(&s.snapshot()?, out, &id),
            Command::Forget { id } => forget(&s, out, &id),
            Command::Config { sets } => config(&s, out, &sets),
            Command::Sync => sync_cmd(&s, out),
            Command::Import { file } => import(&s, out, &file),
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

/// A printed id, `#` optional: a position `7` is `[7, 8)`, a block `16-31` is `[16, 32)`.
pub fn span(s: &str) -> Result<Block> {
    let digits = |p: &str| !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit());
    let id = s.strip_prefix('#').unwrap_or(s);
    let parsed = Some(id.split_once('-').unwrap_or((id, id)))
        .filter(|(a, b)| digits(a) && digits(b))
        .and_then(|(a, b)| {
            Some((
                a.parse::<u64>().ok()?,
                b.parse::<u64>().ok()?.checked_add(1)?,
            ))
        });
    let Some((lo, hi)) = parsed else {
        bail!("'{s}' is not an id. Copy it from the prompt.");
    };
    let n = hi.saturating_sub(lo);
    if n == 0 || !n.is_power_of_two() || lo % n != 0 {
        bail!("{s} is not a block. Copy the id printed by wake, like 16-31.");
    }
    Ok((lo, hi))
}

fn block_id(s: &str) -> Result<Block> {
    let b = span(s)?;
    if b.1 - b.0 < 2 {
        bail!("{s} is not a block. Copy the id printed by wake, like 16-31.");
    }
    Ok(b)
}

pub fn name((lo, hi): Block) -> String {
    match hi - lo {
        1 => lo.to_string(),
        _ => format!("{lo}-{}", hi - 1),
    }
}

fn within(s: &Snapshot, (lo, hi): Block) -> Result<u64> {
    let t = s.log_len()?;
    if lo >= t {
        bail!(
            "#{} is beyond the memory: it holds {}. Run: {ME} wake",
            name((lo, hi)),
            plural(t, "memory")
        );
    }
    Ok(t)
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
        writeln!(out, "Found {at}: {}.", plural(n, "memory"))?;
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

fn wake(s: &Snapshot, out: &mut dyn Write, k: u64, t: Option<u64>) -> Result<ExitCode> {
    let now = s.log_len()?;
    let t = match t {
        Some(t) if t > now => bail!(
            "T={t}, but the log holds {}. Run: {ME} wake",
            plural(now, "memory")
        ),
        Some(t) => t,
        None => now,
    };
    if t == 0 {
        writeln!(
            out,
            "No memories yet. Record the first with: {ME} note \"<one line>\""
        )?;
        writeln!(out, "You are awake.")?;
        return Ok(ExitCode::SUCCESS);
    }
    let mut lines = Vec::new();
    for (lo, hi) in cover(t, s.cfg().get(Knob::WakeLines)) {
        if hi - lo == 1 {
            lines.push(s.log_get(lo)?.line(lo));
            continue;
        }
        let mut sum = s.tree_get(lo, hi)?;
        if sum.is_none() {
            if let Some(nap) = next_nap(s, t)? {
                writeln!(
                    out,
                    "Cannot wake: the memory context needs #{lo}-{}, which is not compressed yet.\n\
                     Do the {} below, then run {ME} wake again.\n\n{nap}",
                    hi - 1,
                    plural(pending_count(s, t)?, "compression"),
                )?;
                return Ok(ExitCode::FAILURE);
            }
            sum = s.tree_get(lo, hi)?;
        }
        let Some(sum) = sum else {
            bail!(blank(lo, hi));
        };
        lines.push(format!("#{lo}-{} {}", hi - 1, sum.text));
    }
    let parts = paginate(
        lines,
        s.cfg().get(Knob::PartLines),
        s.cfg().get(Knob::PartChars),
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
            plural(t, "memory")
        )?;
    }
    writeln!(out, "{}", parts[k as usize - 1].join("\n"))?;
    if k < n {
        writeln!(out, "Not awake yet. Run: {ME} wake {} {t}", k + 1)?;
    } else {
        writeln!(out, "You are awake.")?;
        if let Some(nap) = next_nap(s, t)? {
            writeln!(out, "\n{nap}")?;
        }
    }
    Ok(ExitCode::SUCCESS)
}

fn note(s: &Store, out: &mut dyn Write, text: &str) -> Result<ExitCode> {
    let text = check(text, s.cfg.get(Knob::EntryChars))?;
    let cwd = std::env::current_dir().context("Cannot read the current directory.")?;
    let m = Memory {
        ts: now(),
        place: prov::place(&cwd),
        who: prov::env_who(),
        text: text.into(),
        ..Memory::default()
    };
    let (i, snap) = s.log_append("note", &[m])?;
    writeln!(out, "Saved as #{i}.")?;
    let ts = snap.log_get(i)?.ts;
    if ts > later(&now(), 1.day())? {
        writeln!(
            out,
            "Warning: dated {ts}, over a day ahead of this clock. Check the clocks of every machine."
        )?;
    }
    if let Some(nap) = next_nap(&snap, i + 1)? {
        writeln!(out, "\n{nap}")?;
    }
    Ok(ExitCode::SUCCESS)
}

fn nap(s: &Store, out: &mut dyn Write, given: Option<(&str, &str)>) -> Result<ExitCode> {
    let mut snap = s.snapshot()?;
    let t = snap.log_len()?;
    if let Some((given_id, text)) = given {
        let (id, fp) = given_id.split_once('@').unwrap_or((given_id, ""));
        let fp = &fp.to_ascii_lowercase();
        if !fp.bytes().all(|b| b.is_ascii_hexdigit()) || given_id.ends_with('@') {
            bail!("'{given_id}' is not a block id. Copy it from the prompt.");
        }
        let (lo, hi) = block_id(id)?;
        let changed = || anyhow::anyhow!("{id}: block changed by a sync. Run: {ME} nap");
        if hi <= t && !snap.fingerprint(lo, hi)?.starts_with(fp) {
            return Err(changed());
        }
        let Some(&next) = pending(&snap, t, Some(1))?.first() else {
            writeln!(out, "Nothing left to compress.")?;
            return Ok(ExitCode::SUCCESS);
        };
        let last = hi - 1;
        if (lo, hi) != next {
            if snap.tree_get(lo, hi)?.is_none() {
                bail!(
                    "Wrong block: {id}. Blocks are built in order; the next is {}-{}. Run: {ME} nap",
                    next.0,
                    next.1 - 1
                );
            }
            writeln!(out, "{lo}-{last} is already settled.")?;
        } else {
            let text = check(text, s.cfg.get(Knob::EntryChars))?;
            let (put, after) = s.tree_put((lo, hi), fp, text, &prov::env_who())?;
            snap = after;
            match put {
                Put::Saved => writeln!(out, "{lo}-{last} saved.")?,
                Put::Moved => writeln!(out, "{lo}-{last} was settled or forgotten meanwhile.")?,
                Put::Changed => return Err(changed()),
            }
        }
    }
    match next_nap(&snap, t)? {
        Some(nap) => writeln!(out, "{}{nap}", if given.is_some() { "\n" } else { "" })?,
        None => writeln!(out, "Nothing left to compress.")?,
    }
    Ok(ExitCode::SUCCESS)
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

fn sync_cmd(s: &Store, out: &mut dyn Write) -> Result<ExitCode> {
    let Some(r) = remote(s) else {
        writeln!(
            out,
            "No remote: this memory is local only. To sync it, run: git --git-dir {} remote add origin <url>",
            pretty(s.dir())
        )?;
        return Ok(ExitCode::SUCCESS);
    };
    report(&sync(s, &r, None)?, &r, true, out)?;
    Ok(ExitCode::SUCCESS)
}

fn wake_sync(s: &Store, out: &mut dyn Write) -> Result<()> {
    let Some(r) = remote(s) else {
        return Ok(());
    };
    match sync(s, &r, Some(Instant::now() + WAKE_SYNC)) {
        Ok(rep) => report(&rep, &r, false, out),
        Err(e) => Ok(writeln!(out, "Warning: cannot sync with {r}: {e}")?),
    }
}

fn report(r: &Report, remote: &str, verbose: bool, out: &mut dyn Write) -> Result<()> {
    if let Some(w) = &r.warning {
        writeln!(out, "Warning: {w}.")?;
    }
    if r.clashes > 0 {
        writeln!(
            out,
            "Warning: {} differed between clones under one key; kept one copy each.",
            plural(r.clashes, "memory")
        )?;
    }
    if let Some(t) = &r.taken {
        let mut line = format!("Merged {} from {remote}", plural(t.memories, "memory"));
        if let Some(p) = t.renumbered {
            line += &format!("; positions from #{p} renumbered");
        }
        if t.redo > 0 {
            line += &format!("; {} to redo", plural(t.redo, "summary"));
        }
        line += ".";
        if t.redo > 0 && verbose {
            line += &format!(" Run: {ME} nap");
        }
        writeln!(out, "{line}")?;
    }
    if verbose && r.pushed {
        writeln!(out, "Pushed to {remote}.")?;
    }
    if verbose && r.warning.is_none() && r.taken.is_none() && !r.pushed {
        writeln!(out, "Up to date with {remote}.")?;
    }
    Ok(())
}

fn forget(s: &Store, out: &mut dyn Write, id: &str) -> Result<ExitCode> {
    let (lo, hi) = block_id(id)?;
    let (gone, _) = s.tree_drop(lo, hi)?;
    let Some(&(a, b)) = gone.first() else {
        bail!("No summary at {id}.");
    };
    let n = plural(gone.len() as u64, "summary");
    writeln!(out, "Forgot {n}, from {a}-{} up. Run: {ME} nap", b - 1)?;
    Ok(ExitCode::SUCCESS)
}

fn frontier(s: &Snapshot, (lo, hi): Block, t: u64, depth: u32) -> Result<Vec<String>> {
    let step = match (hi - lo, (hi - lo) >> depth) {
        (n, _) if n <= RAW_MAX => 1,
        (_, s) if s <= 2 => 1,
        (_, s) => s,
    };
    let mut lines = Vec::new();
    for a in (lo..hi.min(t)).step_by(step as usize) {
        let b = a + step;
        lines.push(match step {
            1 => s.log_get(a)?.detail(a),
            _ => match s.tree_get(a, b)? {
                Some(sum) => format!("#{} {}", name((a, b)), sum.text),
                None => format!("#{} not compressed yet", name((a, b))),
            },
        });
    }
    Ok(lines)
}

fn zoom(s: &Snapshot, out: &mut dyn Write, id: &str, depth: u32) -> Result<ExitCode> {
    let b = span(id)?;
    let t = within(s, b)?;
    let cap = s.cfg().get(Knob::PartChars);
    let bytes = |l: &[String]| l.iter().map(|x| x.len() as u64 + 1).sum::<u64>();
    let fits = |l: &[String]| l.len() <= ZOOM_LINES && bytes(l) <= cap;
    let lines = frontier(s, b, t, depth)?;
    if !fits(&lines) {
        for d in (1..depth).rev() {
            if fits(&frontier(s, b, t, d)?) {
                bail!(
                    "Too large at depth {depth}: {} lines, {} bytes; the cap is {ZOOM_LINES} lines, \
                     {cap} bytes: use --depth {d}. Run: {ME} zoom {} --depth {d}",
                    lines.len(),
                    bytes(&lines),
                    name(b)
                );
            }
        }
    }
    writeln!(out, "{}", lines.join("\n"))?;
    Ok(ExitCode::SUCCESS)
}

fn show(s: &Snapshot, out: &mut dyn Write, id: &str) -> Result<ExitCode> {
    let (lo, hi) = span(id)?;
    within(s, (lo, hi))?;
    let fields: Vec<(&str, String)> = if hi - lo == 1 {
        let m = s.log_get(lo)?;
        let (p, w) = (m.place, m.who);
        vec![
            ("ts", m.ts),
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
        let Some(sum) = s.tree_get(lo, hi)? else {
            bail!(
                "#{} is not compressed yet. Run: {ME} zoom {}",
                name((lo, hi)),
                name((lo, hi))
            );
        };
        let w = sum.who;
        vec![
            ("ts", sum.ts),
            ("origin", sum.origin),
            ("fp", sum.fp),
            ("agent", w.agent),
            ("model", w.model),
            ("session", w.session),
            ("text", sum.text),
        ]
    };
    writeln!(out, "#{}", name((lo, hi)))?;
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
    let limit = s.cfg.get(Knob::EntryChars);
    let snap = s.snapshot()?;
    let n = snap.log_len()?;
    let mut last = if n > 0 {
        snap.log_get(n - 1)?.date()
    } else {
        "0000-00-00".into()
    };
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
            bail!("line {i}: date {date} precedes the previous memory ({last}).");
        }
        let text = text.trim();
        if text.is_empty() || text.len() as u64 > limit {
            bail!("line {i}: {} bytes, limit {limit}.", text.len());
        }
        last = date.to_string();
        items.push(Memory {
            ts: midnight(day),
            text: text.into(),
            ..Memory::default()
        });
    }
    if items.is_empty() {
        bail!("{} has no memories.", file.display());
    }
    let (base, snap) = s.log_append("import", &items)?;
    let k = items.len() as u64;
    writeln!(
        out,
        "Imported {}, #{base} to #{}.",
        plural(k, "memory"),
        base + k - 1
    )?;
    let n = pending_count(&snap, snap.log_len()?)?;
    if n > 0 {
        writeln!(out, "{} pending. Run: {ME} nap", plural(n, "compression"))?;
    }
    Ok(ExitCode::SUCCESS)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plurals() {
        assert_eq!(plural(1, "memory"), "1 memory");
        assert_eq!(plural(2, "memory"), "2 memories");
        assert_eq!(plural(5, "match"), "5 matches");
        assert_eq!(plural(0, "part"), "0 parts");
    }

    #[test]
    fn block_ids() {
        assert_eq!(block_id("16-31").unwrap(), (16, 32));
        assert_eq!(block_id("#16-31").unwrap(), (16, 32));
        for bad in ["3-9", "9-3", "4-4", "5-6", "x-1", "1-", "17-32", "7", "#"] {
            assert!(block_id(bad).is_err(), "{bad}");
        }
        assert_eq!(span("7").unwrap(), (7, 8));
        assert_eq!(span("#7").unwrap(), (7, 8));
        assert_eq!((name((7, 8)), name((16, 32))), ("7".into(), "16-31".into()));
    }

    #[test]
    fn pages_respect_both_caps() {
        let lines: Vec<_> = (0..10).map(|i| format!("line {i}")).collect();
        assert_eq!(paginate(lines.clone(), 3, 1000).len(), 4);
        assert_eq!(paginate(lines, 100, 14).len(), 10 / 2);
    }
}
