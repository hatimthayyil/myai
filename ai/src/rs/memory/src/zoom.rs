use anyhow::{Result, bail};

use crate::{meta::Meta, store::Snapshot, tree::Coord, view::HIDDEN};

/// Line `id+n` opened two levels down: the four lines of `n/4` under it, its messages' lines
/// for `n` of 2 or 4, or for `n = 1` the message whole, each with `meta`'s head if given.
/// `None` when `id+n` is not a built line.
pub fn zoom(s: &Snapshot, id: u64, n: u64, meta: Option<&Meta>) -> Result<Option<String>> {
    let Some(c) = Coord::at(id, n, s.log_len()?) else {
        return Ok(None);
    };
    let head = |c| meta.map_or(Ok(String::new()), |m| m.head_at(s, c));
    if c.l == 0 {
        let m = s.message(id)?;
        let h = meta.map_or(String::new(), |h| h.head(std::slice::from_ref(&m)));
        return Ok(Some(format!("{id}+0|{h}{}", m.label())));
    }
    if s.node(c)?.is_none() {
        return Ok(None);
    }
    let l = c.l.saturating_sub(2);
    let mut lines = Vec::new();
    for k in (c.id() >> l..c.end() >> l).map(|i| Coord::new(l, i)) {
        let Some(node) = s.node(k)? else {
            return Ok(None);
        };
        let text = if node.text.is_empty() { HIDDEN } else { &node.text };
        lines.push(format!("{k}|{}{text}", head(k)?));
    }
    Ok(Some(lines.join("\n")))
}

/// `text` in parts of at most `max` bytes, each cut after its last line end when that keeps
/// at least half of it, else at a character boundary.
pub fn pages(text: &str, max: u64) -> Vec<&str> {
    let max = max.max(1) as usize;
    let (mut out, mut rest) = (Vec::new(), text);
    while rest.len() > max {
        let cut = rest.floor_char_boundary(max);
        let at = match rest[..cut].rfind('\n') {
            Some(k) if k + 1 >= cut / 2 => k + 1,
            _ if cut == 0 => rest.ceil_char_boundary(1),
            _ => cut,
        };
        out.push(&rest[..at]);
        rest = &rest[at..];
    }
    if !rest.is_empty() || out.is_empty() {
        out.push(rest);
    }
    out
}

/// Part `part` (from 1) of `text` paged by [`pages`], and how many parts there are.
pub fn page(text: &str, max: u64, part: u64) -> Result<(&str, u64)> {
    let all = pages(text, max);
    let n = all.len() as u64;
    match part.checked_sub(1).and_then(|k| all.get(k as usize)) {
        Some(p) => Ok((p, n)),
        None => bail!("No part {part}: it has {n}."),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pages_cut_at_line_ends_and_characters() {
        assert_eq!(pages("abc", 10), ["abc"]);
        assert_eq!(pages("ab\ncdef\ngh", 8), ["ab\ncdef\n", "gh"]);
        assert_eq!(pages("a\nbcdefgh", 6), ["a\nbcde", "fgh"]);
        assert_eq!(pages("ééé", 3), ["é", "é", "é"]);
        assert_eq!(pages("ééé", 1), ["é", "é", "é"]);
        assert_eq!(page("ab\ncd", 3, 2).unwrap(), ("cd", 2));
        assert!(page("ab", 3, 2).is_err() && page("ab", 3, 0).is_err());
    }
}
