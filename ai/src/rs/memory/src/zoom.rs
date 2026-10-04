use anyhow::Result;

use crate::{store::Snapshot, tree::Coord};

/// Line `id+n` opened: the two lines of `n/2` under it, or for `n = 1` the message whole.
/// `None` when `id+n` is not a built line.
pub fn zoom(s: &Snapshot, id: u64, n: u64) -> Result<Option<String>> {
    let Some(c) = Coord::at(id, n, s.log_len()?) else {
        return Ok(None);
    };
    if c.l == 0 {
        return Ok(Some(format!("{id}+0|{}", s.message(id)?.label())));
    }
    if s.node(c)?.is_none() {
        return Ok(None);
    }
    let mut lines = Vec::new();
    for k in c.children() {
        let Some(node) = s.node(k)? else {
            return Ok(None);
        };
        lines.push(format!("{k}|{}", node.text));
    }
    Ok(Some(lines.join("\n")))
}
