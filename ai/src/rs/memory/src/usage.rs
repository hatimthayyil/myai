use std::{
    fs::OpenOptions,
    io::{self, Write},
    path::Path,
};

use serde::{Deserialize, Serialize};

use crate::record::{Place, Who, now};

pub const USAGE_LOG: &str = "usage.jsonl";

/// One command run against the memory: what, how it went, by whom and from where.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Use {
    pub ts: String,
    pub cmd: String,
    pub args: Vec<String>,
    pub ok: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    pub lines: u64,
    pub bytes: u64,
    pub agent: String,
    pub model: String,
    pub session: String,
    pub repo: String,
    pub branch: String,
}

fn dash(v: &str) -> String {
    if v.is_empty() { "-".into() } else { v.into() }
}

impl Use {
    /// `cmd args`, run now by `who` at `place`, with what it printed and its error.
    pub fn new(
        cmd: &str,
        args: Vec<String>,
        place: &Place,
        who: &Who,
        out: &Tally,
        error: Option<String>,
    ) -> Use {
        Use {
            ts: now(),
            cmd: cmd.into(),
            args,
            ok: error.is_none(),
            error,
            lines: out.lines,
            bytes: out.bytes,
            agent: dash(&who.agent),
            model: dash(&who.model),
            session: dash(&who.session),
            repo: dash(&place.repo),
            branch: dash(&place.branch),
        }
    }
}

/// Appends `u` to the usage log in `dir` as one line, in one write.
pub fn record(dir: &Path, u: &Use) -> io::Result<()> {
    let mut line = serde_json::to_vec(u)?;
    line.push(b'\n');
    OpenOptions::new()
        .create(true)
        .append(true)
        .open(dir.join(USAGE_LOG))?
        .write_all(&line)
}

/// Every readable line of the usage log in `dir`; none if it is absent.
pub fn read(dir: &Path) -> io::Result<Vec<Use>> {
    let text = match std::fs::read_to_string(dir.join(USAGE_LOG)) {
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        r => r?,
    };
    Ok(text
        .lines()
        .filter_map(|l| serde_json::from_str(l).ok())
        .collect())
}

/// A writer that counts the bytes and lines passed through it.
pub struct Tally<'a> {
    out: &'a mut dyn Write,
    pub bytes: u64,
    pub lines: u64,
}

impl Tally<'_> {
    pub fn new(out: &mut dyn Write) -> Tally<'_> {
        Tally {
            out,
            bytes: 0,
            lines: 0,
        }
    }
}

impl Write for Tally<'_> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let n = self.out.write(buf)?;
        self.bytes += n as u64;
        self.lines += memchr::memchr_iter(b'\n', &buf[..n]).count() as u64;
        Ok(n)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.out.flush()
    }
}
