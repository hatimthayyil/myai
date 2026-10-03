use anyhow::{Result, bail};

use crate::{
    config::Knob,
    cover::Block,
    store::{ME, Store},
};

const RAW_MAX: u64 = 16;

/// Blocks buildable from `t` memories and not built yet, smallest first.
pub fn pending(s: &Store, t: u64, limit: Option<usize>) -> Result<Vec<Block>> {
    let (mut todo, mut size) = (Vec::new(), 2);
    while size <= t {
        for k in s.level_len(size)?..t / size {
            todo.push((k * size, (k + 1) * size));
            if limit.is_some_and(|l| todo.len() >= l) {
                return Ok(todo);
            }
        }
        size *= 2;
    }
    Ok(todo)
}

/// How many blocks [`pending`] would list, without listing them.
pub fn pending_count(s: &Store, t: u64) -> Result<u64> {
    let (mut n, mut size) = (0, 2);
    while size <= t {
        n += (t / size).saturating_sub(s.level_len(size)?);
        size *= 2;
    }
    Ok(n)
}

pub fn blank(lo: u64, hi: u64) -> anyhow::Error {
    let id = format!("{lo}-{}", hi - 1);
    anyhow::anyhow!("The summary of #{id} is blank. Run: {ME} forget {id}")
}

fn prompt(s: &Store, (lo, hi): Block, left: u64) -> Result<String> {
    let body = if hi - lo <= RAW_MAX {
        s.log_slice(lo, hi)?
            .iter()
            .map(|e| format!("  {e}"))
            .collect::<Vec<_>>()
    } else {
        let mid = (lo + hi) / 2;
        let mut halves = Vec::new();
        for (a, b) in [(lo, mid), (mid, hi)] {
            let Some(sum) = s.tree_get(a, b)? else {
                bail!(blank(a, b));
            };
            halves.push(format!("  #{a}-{} {sum}", b - 1));
        }
        halves
    };
    let tail = match left {
        0 => String::new(),
        1 => "\n1 compression remains after this one.".into(),
        n => format!("\n{n} compressions remain after this one."),
    };
    Ok(format!(
        "Compress memories #{lo}-{last} into one line of at most {chars} bytes.\n\
         Keep what has lasting effect, drop what does not. Invent nothing.\n\n\
         {body}\n{tail}\n\
         Run: {ME} nap {lo}-{last} \"<your line>\"",
        last = hi - 1,
        chars = s.cfg.get(Knob::EntryChars),
        body = body.join("\n"),
    ))
}

/// The prompt for the next pending compression as of `t`, if any.
pub fn next_nap(s: &Store, t: u64) -> Result<Option<String>> {
    match pending(s, t, Some(1))?.first() {
        Some(&b) => Ok(Some(prompt(s, b, pending_count(s, t)? - 1)?)),
        None => Ok(None),
    }
}
