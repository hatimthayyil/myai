use std::{
    collections::VecDeque,
    fs,
    io::{self, Write},
    path::{Path, PathBuf},
    process::ExitCode,
};

use anyhow::{Context, Result, bail};
use clap::{Parser, Subcommand};
use regex::RegexBuilder;

use crate::{
    config::Knob,
    cover::{Block, cover},
    nap::{blank, next_nap, pending, pending_count},
    repo,
    store::{AtPath, ME, Store, pretty},
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

Do not register redundant memories.

If `{memo} note` asks a compression: do it before your next action.

Never edit or delete anything under `{data}`: the tool manages it.

### When you need an old memory: search, or navigate

`{memo} recall <regex>` searches every memory, word for word.

Your memories also form a binary tree: #0-1, #2-3 ... exist as one-line
summaries, pairs of those as #0-3, and so on -- every `#a-b` line wake
prints is one node of it. `{memo} zoom <a-b>` opens a node into its
two halves, down to the raw memories.

### If you're a subagent: skip everything above

Parallel sessions in this repository are all you, and may all write memories.
A subagent is not: it must never run `{memo}`, because it cannot judge what
is already known, and its notes would arrive duplicated and incorrectly.
When you spawn one, write: `You are a subagent. Don't run {memo}.`";

#[derive(Parser, Debug)]
#[command(
    name = "memory",
    about = "A permanent, append-only memory for AI agents.",
    after_help = "The memories live in <repo root>/.ai/memory, or in $AI_MEMORY_DIR if set."
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
    #[command(about = "search every memory ever recorded.")]
    Recall {
        #[arg(allow_hyphen_values = true)]
        regex: String,
    },
    #[command(about = "open a tree node: its two halves.")]
    Zoom { id: String },
    #[command(about = "drop a bad summary; nap rebuilds it.")]
    Forget { id: String },
    #[command(about = "show this memory's sizes, or change one.")]
    Config {
        #[arg(value_name = "NAME=VALUE")]
        sets: Vec<String>,
    },
    #[command(about = "bulk-load dated memories (bootstrap only).")]
    Import { file: PathBuf },
}

/// Where the memory lives, and the repository it belongs to when it is the default.
pub struct Location {
    pub dir: PathBuf,
    pub repo: Option<PathBuf>,
}

impl Location {
    /// `$AI_MEMORY_DIR`, else `<repo root>/.ai/memory`.
    pub fn find() -> Result<Location> {
        if let Some(d) = std::env::var_os("AI_MEMORY_DIR").filter(|d| !d.is_empty()) {
            return Ok(Location {
                dir: d.into(),
                repo: None,
            });
        }
        let cwd = std::env::current_dir().context("Cannot read the current directory.")?;
        let Some(root) = repo::root(&cwd) else {
            bail!(
                "Not in a git repository. Run inside a repo, or point AI_MEMORY_DIR at a memory."
            );
        };
        Ok(Location {
            dir: root.join(repo::STORE),
            repo: Some(root.to_path_buf()),
        })
    }
}

impl Cli {
    /// Runs against the default memory, printing to stdout.
    pub fn exec(self) -> Result<ExitCode> {
        match self.run(&Location::find()?, &mut io::stdout().lock()) {
            Err(e) if broken_pipe(&e) => Ok(ExitCode::SUCCESS),
            r => r,
        }
    }

    pub fn run(self, at: &Location, out: &mut dyn Write) -> Result<ExitCode> {
        if let Command::Init = self.command {
            return init(at, out);
        }
        let s = Store::open(&at.dir)?;
        match self.command {
            Command::Init => unreachable!(),
            Command::Wake { part, t } => wake(&s, out, part.unwrap_or(1), t),
            Command::Note { text } => note(&s, out, &text),
            Command::Nap { id, text } => nap(&s, out, id.as_deref().zip(text.as_deref())),
            Command::Recall { regex } => recall(&s, out, &regex),
            Command::Zoom { id } => zoom(&s, out, &id),
            Command::Forget { id } => forget(&s, out, &id),
            Command::Config { sets } => config(&s, out, &sets),
            Command::Import { file } => import(&s, out, &file),
        }
    }
}

fn broken_pipe(e: &anyhow::Error) -> bool {
    e.chain()
        .filter_map(|c| c.downcast_ref::<io::Error>())
        .any(|e| e.kind() == io::ErrorKind::BrokenPipe)
}

fn plural(n: u64, word: &str) -> String {
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

fn block_id(s: &str) -> Result<Block> {
    let digits = |p: &str| !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit());
    let parsed = s
        .split_once('-')
        .filter(|(a, b)| digits(a) && digits(b))
        .and_then(|(a, b)| {
            Some((
                a.parse::<u64>().ok()?,
                b.parse::<u64>().ok()?.checked_add(1)?,
            ))
        });
    let Some((lo, hi)) = parsed else {
        bail!("'{s}' is not a block id. Copy it from the prompt.");
    };
    let n = hi.saturating_sub(lo);
    if n < 2 || !n.is_power_of_two() || lo % n != 0 {
        bail!("{s} is not a block. Copy the id printed by wake, like 16-31.");
    }
    Ok((lo, hi))
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

fn init(loc: &Location, out: &mut dyn Write) -> Result<ExitCode> {
    let (s, fresh) = Store::create(&loc.dir)?;
    let at = pretty(&loc.dir);
    if fresh {
        writeln!(out, "Created {at}: this repository's memory.")?;
    } else {
        writeln!(out, "Found {at}: {}.", plural(s.log_len()?, "memory"))?;
    }
    if let Some(root) = &loc.repo
        && repo::ignore_store(root)?
    {
        writeln!(out, "Added /.ai/memory/ to {}/.gitignore.", pretty(root))?;
    }
    writeln!(out, "Sizes live in {at}/config; the defaults are fine.\n")?;
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

fn wake(s: &Store, out: &mut dyn Write, k: u64, t: Option<u64>) -> Result<ExitCode> {
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
    for (lo, hi) in cover(t, s.cfg.get(Knob::WakeLines)) {
        if hi - lo == 1 {
            lines.push(s.log_get(lo)?.to_string());
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
        lines.push(format!("#{lo}-{} {sum}", hi - 1));
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
    let today = jiff::Zoned::now().date().to_string();
    let i = s.log_append(&[(today, text.to_string())])?;
    writeln!(out, "Saved as #{i}.")?;
    if let Some(nap) = next_nap(s, i + 1)? {
        writeln!(out, "\n{nap}")?;
    }
    Ok(ExitCode::SUCCESS)
}

fn nap(s: &Store, out: &mut dyn Write, given: Option<(&str, &str)>) -> Result<ExitCode> {
    let t = s.log_len()?;
    if let Some((id, text)) = given {
        let (lo, hi) = block_id(id)?;
        let Some(&next) = pending(s, t, Some(1))?.first() else {
            writeln!(out, "Nothing left to compress.")?;
            return Ok(ExitCode::SUCCESS);
        };
        let last = hi - 1;
        if (lo, hi) != next {
            if s.tree_get(lo, hi)?.is_none() {
                bail!(
                    "Wrong block: {id}. Blocks are built in order; the next is {}-{}. Run: {ME} nap",
                    next.0,
                    next.1 - 1
                );
            }
            writeln!(out, "{lo}-{last} is already settled.")?;
        } else if !s.tree_put(lo, hi, check(text, s.cfg.get(Knob::EntryChars))?)? {
            writeln!(out, "{lo}-{last} was settled or forgotten meanwhile.")?;
        } else {
            writeln!(out, "{lo}-{last} saved.")?;
        }
    }
    match next_nap(s, t)? {
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
                Some(k.validate(v, "")?)
            },
        );
    }
    if !sets.is_empty() {
        cfg.write(s.dir())?;
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

fn forget(s: &Store, out: &mut dyn Write, id: &str) -> Result<ExitCode> {
    let (lo, hi) = block_id(id)?;
    let gone = s.tree_drop(lo, hi)?;
    let Some(&(a, b)) = gone.first() else {
        bail!("No summary at {id}.");
    };
    let n = plural(gone.len() as u64, "summary");
    writeln!(out, "Forgot {n}, from {a}-{} up. Run: {ME} nap", b - 1)?;
    Ok(ExitCode::SUCCESS)
}

fn recall(s: &Store, out: &mut dyn Write, regex: &str) -> Result<ExitCode> {
    let pat = match RegexBuilder::new(regex).case_insensitive(true).build() {
        Ok(p) => p,
        Err(e) => bail!("bad regex: {e}"),
    };
    let cap = s.cfg.get(Knob::PartChars);
    let (mut hits, mut kept, mut size) = (0, VecDeque::new(), 0);
    s.log_scan(|e| {
        let line = e.to_string();
        if pat.is_match(&line) {
            hits += 1;
            size += line.len() as u64 + 1;
            kept.push_back(line);
            while size > cap {
                size -= kept.pop_front().map_or(0, |l: String| l.len() as u64 + 1);
            }
        }
        Ok(())
    })?;
    if hits == 0 {
        writeln!(out, "No match.")?;
        return Ok(ExitCode::SUCCESS);
    }
    for line in &kept {
        writeln!(out, "{line}")?;
    }
    if (kept.len() as u64) < hits {
        writeln!(
            out,
            "Newest {} of {}. Narrow the regex.",
            kept.len(),
            plural(hits, "match")
        )?;
    } else {
        writeln!(out, "{}.", plural(hits, "match"))?;
    }
    Ok(ExitCode::SUCCESS)
}

fn zoom(s: &Store, out: &mut dyn Write, id: &str) -> Result<ExitCode> {
    let (lo, hi) = block_id(id)?;
    let t = s.log_len()?;
    if lo >= t {
        bail!(
            "#{id} is beyond the memory: it holds {}. Run: {ME} wake",
            plural(t, "memory")
        );
    }
    let mid = (lo + hi) / 2;
    for (a, b) in [(lo, mid), (mid, hi)] {
        if a >= t {
            continue;
        }
        if b - a == 1 {
            writeln!(out, "{}", s.log_get(a)?)?;
        } else {
            let sum = s.tree_get(a, b)?;
            writeln!(
                out,
                "#{a}-{} {}",
                b - 1,
                sum.as_deref().unwrap_or("not compressed yet")
            )?;
        }
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
    let n = s.log_len()?;
    let mut last = if n > 0 {
        s.log_get(n - 1)?.date
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
        if date.parse::<jiff::civil::Date>().is_err() {
            bail!("line {i}: {date} is not a real date.");
        }
        if *date < *last {
            bail!("line {i}: date {date} precedes the previous memory ({last}).");
        }
        let text = text.trim();
        if text.is_empty() || text.len() as u64 > limit {
            bail!("line {i}: {} bytes, limit {limit}.", text.len());
        }
        last = date.to_string();
        items.push((last.clone(), text.to_string()));
    }
    if items.is_empty() {
        bail!("{} has no memories.", file.display());
    }
    let base = s.log_append(&items)?;
    let k = items.len() as u64;
    writeln!(
        out,
        "Imported {}, #{base} to #{}.",
        plural(k, "memory"),
        base + k - 1
    )?;
    let n = pending_count(s, s.log_len()?)?;
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
        for bad in ["3-9", "9-3", "4-4", "5-6", "x-1", "1-", "17-32"] {
            assert!(block_id(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn pages_respect_both_caps() {
        let lines: Vec<_> = (0..10).map(|i| format!("line {i}")).collect();
        assert_eq!(paginate(lines.clone(), 3, 1000).len(), 4);
        assert_eq!(paginate(lines, 100, 14).len(), 10 / 2);
    }
}
