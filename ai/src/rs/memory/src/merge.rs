use std::collections::HashMap;

use anyhow::Result;
use gix::ObjectId;

use crate::{
    record::{Memory, REC, Summary, fingerprint},
    store::{Changes, LOG, Snapshot, Store, level},
};

/// Merges commit `theirs` into the memory; returns how many memories differed under one key.
pub fn merge(s: &Store, theirs: ObjectId, msg: &str) -> Result<u64> {
    loop {
        let ours = s.head()?;
        if let Some(o) = ours
            && s.is_ancestor(theirs, o)?
        {
            return Ok(0);
        }
        let (a, b) = (s.at(ours)?, s.at(Some(theirs))?);
        let (to, clashes) = match ours {
            Some(o) if !s.is_ancestor(o, theirs)? && a.tree() != b.tree() => {
                let (ch, clashes) = union(&a, &b)?;
                (s.commit(&a, ch, msg, [o, theirs])?, clashes)
            }
            _ => (theirs, 0),
        };
        if s.cas(ours, to, msg)? {
            return Ok(clashes);
        }
    }
}

/// The first position of `a` whose memory is not at that position in `b`.
pub fn first_change(a: &Snapshot, b: &Snapshot) -> Result<Option<u64>> {
    let (x, y) = (a.dump(LOG)?, b.dump(LOG)?);
    let (x, y) = (keyed(&x)?, keyed(&y)?);
    Ok(x.iter()
        .enumerate()
        .find(|&(i, m)| y.get(i).is_none_or(|n| n.0 != m.0))
        .map(|(i, _)| i as u64))
}

fn keyed(buf: &[u8]) -> Result<Vec<(String, &[u8])>> {
    buf.chunks(REC)
        .map(|r| Ok((Memory::decode(r)?.key(), r)))
        .collect()
}

fn union(a: &Snapshot, b: &Snapshot) -> Result<(Changes, u64)> {
    let (x, y) = (a.dump(LOG)?, b.dump(LOG)?);
    let (mut xs, mut ys) = (
        keyed(&x)?.into_iter().peekable(),
        keyed(&y)?.into_iter().peekable(),
    );
    let (mut log, mut clashes) = (Vec::new(), 0);
    loop {
        let next = match (xs.peek(), ys.peek()) {
            (Some(p), Some(q)) if p.0 < q.0 => xs.next(),
            (Some(p), Some(q)) if p.0 > q.0 => ys.next(),
            (Some(_), Some(_)) => {
                let (p, q) = (xs.next().unwrap(), ys.next().unwrap());
                clashes += u64::from(p.1 != q.1);
                Some(p.min(q))
            }
            (Some(_), None) => xs.next(),
            (None, _) => ys.next(),
        };
        let Some(m) = next else { break };
        log.push(m);
    }
    let mut ch = Changes::default();
    let n = log.len() as u64;
    let bytes: Vec<&[u8]> = log.iter().map(|m| m.1).collect();
    write(a, LOG, &x, &bytes, &mut ch)?;
    let (mut size, mut cap) = (2, n / 2);
    while size <= n {
        let mut pool: HashMap<String, (String, String, &[u8])> = HashMap::new();
        let (ox, oy) = (a.dump(&level(size))?, b.dump(&level(size))?);
        for r in ox.chunks(REC).chain(oy.chunks(REC)) {
            let Some(sum) = Summary::decode(r) else {
                continue;
            };
            let cand = (sum.ts, sum.origin, r);
            match pool.get(&sum.fp) {
                Some(have) if *have >= cand => {}
                _ => {
                    pool.insert(sum.fp, cand);
                }
            }
        }
        let mut built = Vec::new();
        for k in 0..cap.min(n / size) {
            let keys = &log[(k * size) as usize..((k + 1) * size) as usize];
            let fp = fingerprint(keys.iter().map(|m| m.0.as_str()));
            let Some(&(_, _, r)) = pool.get(&fp) else {
                break;
            };
            built.push(r);
        }
        cap = built.len() as u64 / 2;
        write(a, &level(size), &ox, &built, &mut ch)?;
        size *= 2;
    }
    Ok((ch, clashes))
}

/// Rewrites `dir` in `ch` from its first record that differs from `old`.
fn write(s: &Snapshot, dir: &str, old: &[u8], new: &[&[u8]], ch: &mut Changes) -> Result<()> {
    let old: Vec<&[u8]> = old.chunks(REC).collect();
    let from = old
        .iter()
        .zip(new)
        .position(|(o, n)| o != n)
        .unwrap_or(old.len().min(new.len()));
    s.replace(
        dir,
        from as u64,
        old.len() as u64,
        new[from..].iter().map(|r| r.to_vec()),
        ch,
    )
}
