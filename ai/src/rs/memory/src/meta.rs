use std::collections::HashMap;

use anyhow::Result;
use clap::{Args, ValueEnum};

use crate::{record::Message, store::Snapshot, tree::Coord};

#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
pub enum Field {
    Origin,
    Repo,
    Head,
    Branch,
    Agent,
    Model,
    Session,
    Kind,
}

impl Field {
    pub fn name(self) -> &'static str {
        match self {
            Field::Origin => "origin",
            Field::Repo => "repo",
            Field::Head => "head",
            Field::Branch => "branch",
            Field::Agent => "agent",
            Field::Model => "model",
            Field::Session => "session",
            Field::Kind => "kind",
        }
    }

    fn of(self, m: &Message) -> &str {
        match self {
            Field::Origin => &m.origin,
            Field::Repo => &m.place.repo,
            Field::Head => &m.place.head,
            Field::Branch => &m.place.branch,
            Field::Agent => &m.who.agent,
            Field::Model => &m.who.model,
            Field::Session => &m.who.session,
            Field::Kind => m.kind.name(),
        }
    }
}

/// What a line shows of its messages' metadata, between its `id+n|` and its text.
#[derive(Args, Clone, Debug, Default)]
pub struct Meta {
    #[arg(long, help = "add the time of day (UTC) to the dates")]
    pub time: bool,
    #[arg(
        short = 'o',
        long = "fields",
        value_enum,
        value_delimiter = ',',
        value_name = "FIELD,...",
        help = "add these fields; mixed values show as the commonest and (+k) others"
    )]
    pub fields: Vec<Field>,
}

pub fn date(ts: &str) -> String {
    format!("{}-{}-{}", &ts[..4], &ts[4..6], &ts[6..8])
}

fn time(ts: &str) -> String {
    format!("{}:{}", &ts[9..11], &ts[11..13])
}

/// `a..b`, UTC: `2026-10-04..10-05`, one day as `2026-10-08`; with `time`,
/// `2026-10-08 09:12..17:40`. The end drops what it shares with the start, down to the date.
pub fn span(a: &str, b: &str, time_of_day: bool) -> String {
    let at = |ts: &str| match time_of_day {
        true => format!("{} {}", date(ts), time(ts)),
        false => date(ts),
    };
    let (from, to) = (at(a), at(b));
    if from == to {
        return from;
    }
    let end = match (a[..4] == b[..4], a[..8] == b[..8] && time_of_day) {
        (_, true) => time(b),
        (true, _) => to[5..].to_string(),
        _ => to,
    };
    format!("{from}..{end}")
}

/// The one value, else the commonest (a real one before `-`, the earliest of a tie) and
/// `(+k)` for the k others.
fn values<'a>(vs: impl Iterator<Item = &'a str>) -> String {
    let mut count: HashMap<&str, (usize, usize)> = HashMap::new();
    for (k, v) in vs.enumerate() {
        count.entry(v).or_insert((0, k)).0 += 1;
    }
    let top = count
        .iter()
        .max_by_key(|(v, (n, first))| (**v != "-", *n, std::cmp::Reverse(*first)))
        .map_or("-", |(v, _)| v);
    match count.len() {
        0 | 1 => top.into(),
        n => format!("{top}(+{})", n - 1),
    }
}

impl Meta {
    /// `dates|field|…|` of `ms`, the messages a line covers, in order. Only the first and
    /// last are read for the dates: the log is in time order.
    pub fn head(&self, ms: &[Message]) -> String {
        let (Some(a), Some(b)) = (ms.first(), ms.last()) else {
            return String::new();
        };
        let mut s = span(&a.ts, &b.ts, self.time);
        for f in &self.fields {
            s.push('|');
            s += &values(ms.iter().map(|m| f.of(m)));
        }
        s.push('|');
        s
    }

    /// [`Meta::head`] of line `c`, reading only the messages it needs.
    pub fn head_at(&self, s: &Snapshot, c: Coord) -> Result<String> {
        let mut ids: Vec<u64> = match self.fields.is_empty() {
            true => vec![c.id(), c.end() - 1],
            false => (c.id()..c.end()).collect(),
        };
        ids.dedup();
        let ms = ids
            .into_iter()
            .map(|i| s.message(i))
            .collect::<Result<Vec<_>>>()?;
        Ok(self.head(&ms))
    }

    /// The flags that ask for this, as typed.
    pub fn args(&self) -> Vec<String> {
        let mut a = Vec::new();
        if self.time {
            a.push("--time".into());
        }
        if !self.fields.is_empty() {
            let names: Vec<_> = self.fields.iter().map(|f| f.name()).collect();
            a.extend(["-o".into(), names.join(",")]);
        }
        a
    }

    /// [`Meta::args`], each after a space.
    pub fn flags(&self) -> String {
        self.args().iter().map(|a| format!(" {a}")).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spans_drop_what_the_end_shares() {
        let (a, b) = ("20261004T091200Z", "20261005T174000Z");
        assert_eq!(span(a, b, false), "2026-10-04..10-05");
        assert_eq!(span(a, b, true), "2026-10-04 09:12..10-05 17:40");
        assert_eq!(span(a, "20261004T235959Z", false), "2026-10-04");
        assert_eq!(span(a, "20261004T174000Z", true), "2026-10-04 09:12..17:40");
        assert_eq!(span(a, "20261004T091259Z", true), "2026-10-04 09:12");
        assert_eq!(
            span("20251230T000000Z", "20260102T000000Z", false),
            "2025-12-30..2026-01-02"
        );
        assert_eq!(
            span("20260930T235000Z", "20261001T001000Z", true),
            "2026-09-30 23:50..10-01 00:10"
        );
        assert_eq!(
            span("20251231T235000Z", "20260101T001000Z", true),
            "2025-12-31 23:50..2026-01-01 00:10"
        );
    }

    #[test]
    fn many_values_show_the_commonest_and_a_count() {
        assert_eq!(values(["a"].into_iter()), "a");
        assert_eq!(values(["a", "a"].into_iter()), "a");
        assert_eq!(values(["a", "b", "b", "c"].into_iter()), "b(+2)");
        assert_eq!(values(["a", "b"].into_iter()), "a(+1)");
        assert_eq!(values(["-", "-", "a"].into_iter()), "a(+1)");
    }
}
