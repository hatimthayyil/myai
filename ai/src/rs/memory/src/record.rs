use anyhow::{Context, Result, bail};
use jiff::{ToSpan, civil::DateTime, tz::TimeZone};
use sha2::{Digest, Sha256};

pub const REC: usize = 512;
const TS: &str = "%Y%m%dT%H%M%SZ";
const LOG: [usize; 8] = [16, 6, 32, 12, 32, 32, 32, 36];
const TREE: [usize; 6] = [16, 6, 16, 32, 32, 36];

pub const TEXT_MAX: u64 = 305;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Place {
    pub repo: String,
    pub head: String,
    pub branch: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Who {
    pub agent: String,
    pub model: String,
    pub session: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Memory {
    pub ts: String,
    pub origin: String,
    pub place: Place,
    pub who: Who,
    pub text: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Summary {
    pub ts: String,
    pub origin: String,
    pub fp: String,
    pub who: Who,
    pub text: String,
}

fn field(v: &str, width: usize) -> String {
    let v: String = v
        .chars()
        .map(|c| match c {
            c if c.is_whitespace() => '_',
            c if !c.is_ascii() => '?',
            c => c,
        })
        .take(width)
        .collect();
    if v.is_empty() { "-".into() } else { v }
}

fn encode(meta: &[&str], widths: &[usize], text: &str) -> Result<Vec<u8>> {
    let mut b = Vec::with_capacity(REC);
    for (v, &w) in meta.iter().zip(widths) {
        b.extend(format!("{:<w$} ", field(v, w)).bytes());
    }
    let text = text.replace(['\t', '\r'], " ");
    let room = REC - 1 - b.len();
    if text.len() > room {
        bail!("Too long: {} bytes. The record holds {room}.", text.len());
    }
    b.extend(text.bytes());
    b.resize(REC - 1, b' ');
    b.push(b'\n');
    Ok(b)
}

fn decode<const N: usize>(rec: &[u8], widths: [usize; N]) -> Option<([String; N], String)> {
    let mut at = 0;
    let mut meta = widths.map(|w| {
        let s = rec.get(at..at + w);
        at += w + 1;
        s
    });
    let text = std::str::from_utf8(rec.get(at..REC - 1)?).ok()?;
    let mut out: [String; N] = std::array::from_fn(|_| String::new());
    for (o, m) in out.iter_mut().zip(meta.iter_mut()) {
        *o = std::str::from_utf8(m.take()?).ok()?.trim_end().into();
    }
    Some((out, text.trim_end().into()))
}

impl Memory {
    pub fn encode(&self) -> Result<Vec<u8>> {
        let (p, w) = (&self.place, &self.who);
        encode(
            &[
                &self.ts,
                &self.origin,
                &p.repo,
                &p.head,
                &p.branch,
                &w.agent,
                &w.model,
                &w.session,
            ],
            &LOG,
            &self.text,
        )
    }

    pub fn decode(rec: &[u8]) -> Result<Memory> {
        let ([ts, origin, repo, head, branch, agent, model, session], text) = decode(rec, LOG)
            .filter(|(m, _)| m[0].len() == 16 && m[0].is_ascii())
            .context("A log record is corrupt.")?;
        Ok(Memory {
            ts,
            origin,
            place: Place { repo, head, branch },
            who: Who {
                agent,
                model,
                session,
            },
            text,
        })
    }

    pub fn key(&self) -> String {
        format!("{} {}", self.ts, self.origin)
    }

    pub fn date(&self) -> String {
        let t = &self.ts;
        format!("{}-{}-{}", &t[..4], &t[4..6], &t[6..8])
    }

    pub fn line(&self, pos: u64) -> String {
        format!("#{pos} {} {}", self.date(), self.text)
    }

    pub fn detail(&self, pos: u64) -> String {
        let t = &self.ts;
        format!(
            "#{pos} {} {}:{} {} {} {}",
            self.date(),
            &t[9..11],
            &t[11..13],
            self.origin,
            self.place.repo,
            self.text
        )
    }
}

impl Summary {
    pub fn encode(&self) -> Result<Vec<u8>> {
        let w = &self.who;
        encode(
            &[
                &self.ts,
                &self.origin,
                &self.fp,
                &w.agent,
                &w.model,
                &w.session,
            ],
            &TREE,
            &self.text,
        )
    }

    pub fn decode(rec: &[u8]) -> Option<Summary> {
        let ([ts, origin, fp, agent, model, session], text) = decode(rec, TREE)?;
        Some(Summary {
            ts,
            origin,
            fp,
            who: Who {
                agent,
                model,
                session,
            },
            text,
        })
    }
}

pub fn now() -> String {
    ts(jiff::Timestamp::now().to_zoned(TimeZone::UTC).datetime())
}

fn ts(dt: DateTime) -> String {
    dt.strftime(TS).to_string()
}

fn parse_ts(t: &str) -> Result<DateTime> {
    DateTime::strptime(TS, t).with_context(|| format!("'{t}' is not a timestamp."))
}

pub fn midnight(date: jiff::civil::Date) -> String {
    ts(date.to_datetime(jiff::civil::Time::midnight()))
}

pub fn later(t: &str, span: jiff::Span) -> Result<String> {
    Ok(ts(parse_ts(t)?.checked_add(span)?))
}

/// The timestamp a new memory wanting `want` gets after `tail`, keeping keys strictly increasing.
pub fn next_ts(want: &str, tail: Option<(&str, &str)>, origin: &str) -> Result<String> {
    let Some((tail_ts, tail_origin)) = tail else {
        return Ok(want.into());
    };
    let t = want.max(tail_ts);
    if (t, origin) <= (tail_ts, tail_origin) {
        return later(tail_ts, 1.second());
    }
    Ok(t.into())
}

pub fn fingerprint<'a>(keys: impl IntoIterator<Item = &'a str>) -> String {
    let mut h = Sha256::new();
    for k in keys {
        h.update(k.as_bytes());
        h.update(b"\n");
    }
    h.finalize()[..8]
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

pub fn crockford(bits: u32, len: usize) -> String {
    const ABC: &[u8; 32] = b"0123456789ABCDEFGHJKMNPQRSTVWXYZ";
    (0..len)
        .map(|i| ABC[(bits >> (5 * i) & 31) as usize] as char)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn memory() -> Memory {
        Memory {
            ts: "20261003T142233Z".into(),
            origin: "7KQ2ZD".into(),
            place: Place {
                repo: "acme/widget".into(),
                head: "0123456789ab".into(),
                branch: "feature/a b".into(),
            },
            who: Who {
                agent: "claude-code".into(),
                model: "".into(),
                session: "ünï".into(),
            },
            text: "a memory\twith a tab".into(),
        }
    }

    #[test]
    fn layout() {
        let b = memory().encode().unwrap();
        assert_eq!(b.len(), REC);
        assert_eq!(&b[..17], b"20261003T142233Z ");
        assert_eq!(&b[17..24], b"7KQ2ZD ");
        assert_eq!(&b[24..35], b"acme/widget");
        assert_eq!(&b[57..70], b"0123456789ab ");
        assert_eq!(b[206], b'a');
        assert_eq!(b[REC - 1], b'\n');
    }

    #[test]
    fn round_trip() {
        let m = memory();
        let back = Memory::decode(&m.encode().unwrap()).unwrap();
        assert_eq!(back.place.branch, "feature/a_b");
        assert_eq!(back.who.model, "-");
        assert_eq!(back.who.session, "?n?");
        assert_eq!(back.text, "a memory with a tab");
        assert_eq!(back.ts, m.ts);
        assert_eq!(back.line(7), "#7 2026-10-03 a memory with a tab");
        assert_eq!(
            back.detail(7),
            "#7 2026-10-03 14:22 7KQ2ZD acme/widget a memory with a tab"
        );
        assert_eq!(Memory::decode(&back.encode().unwrap()).unwrap(), back);
        let fits = |n: u64| {
            Memory {
                text: "x".repeat(n as usize),
                ..memory()
            }
            .encode()
            .is_ok()
        };
        assert!(fits(TEXT_MAX) && !fits(TEXT_MAX + 1));

        let s = Summary {
            ts: m.ts.clone(),
            origin: m.origin.clone(),
            fp: fingerprint([m.key().as_str()]),
            who: m.who.clone(),
            text: "x".repeat(367),
        };
        let back = Summary::decode(&s.encode().unwrap()).unwrap();
        assert_eq!((back.fp.len(), back.text.len()), (16, 367));
        assert!(
            Summary {
                text: "x".repeat(368),
                ..s
            }
            .encode()
            .is_err()
        );
        assert!(Memory::decode(&[0xff; REC]).is_err());
        assert!(Memory::decode(&[b' '; REC]).is_err());
        let long = Memory {
            place: Place {
                branch: "b".repeat(40),
                ..Place::default()
            },
            ..memory()
        };
        let back = Memory::decode(&long.encode().unwrap()).unwrap();
        assert_eq!(back.place.branch, "b".repeat(32));
    }

    #[test]
    fn keys_only_grow() {
        let (a, b) = ("20260101T000000Z", "20260101T000005Z");
        assert_eq!(next_ts(a, None, "M").unwrap(), a);
        assert_eq!(next_ts(b, Some((a, "M")), "M").unwrap(), b);
        assert_eq!(next_ts(a, Some((b, "M")), "M").unwrap(), "20260101T000006Z");
        assert_eq!(next_ts(a, Some((a, "M")), "M").unwrap(), "20260101T000001Z");
        assert_eq!(next_ts(a, Some((a, "A")), "M").unwrap(), a);
        assert_eq!(next_ts(a, Some((a, "Z")), "M").unwrap(), "20260101T000001Z");
        assert_eq!(
            next_ts(a, Some(("20261231T235959Z", "M")), "M").unwrap(),
            "20270101T000000Z"
        );
    }

    #[test]
    fn fingerprints_and_origins() {
        let fp = fingerprint(["20260101T000000Z AAAAAA"]);
        assert_eq!(fp.len(), 16);
        assert_ne!(fp, fingerprint(["20260101T000000Z AAAAAB"]));
        assert_eq!(fingerprint(std::iter::empty()), "e3b0c44298fc1c14");
        assert_eq!(crockford(0, 6), "000000");
        assert_eq!(crockford(u32::MAX, 6), "ZZZZZZ");
        assert_eq!(crockford(1 | 31 << 5, 6), "1Z0000");
    }
}
