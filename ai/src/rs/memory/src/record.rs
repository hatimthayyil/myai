use anyhow::Result;
use jiff::{civil::DateTime, tz::TimeZone};

const TS: &str = "%Y%m%dT%H%M%SZ";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, clap::ValueEnum)]
pub enum Kind {
    User,
    Ai,
    Tool,
    Note,
}

impl Kind {
    pub const ALL: [Kind; 4] = [Kind::User, Kind::Ai, Kind::Tool, Kind::Note];

    pub fn name(self) -> &'static str {
        match self {
            Kind::User => "user",
            Kind::Ai => "ai",
            Kind::Tool => "tool",
            Kind::Note => "note",
        }
    }

    /// Whether this kind shows in the view: a tool call's line is empty, seen only by zooming.
    pub fn shown(self) -> bool {
        self != Kind::Tool
    }

    pub fn parse(s: &str) -> Option<Kind> {
        Kind::ALL.into_iter().find(|k| k.name() == s)
    }
}

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

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Message {
    pub ts: String,
    pub kind: Kind,
    pub origin: String,
    pub place: Place,
    pub who: Who,
    pub text: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Node {
    pub ts: String,
    pub origin: String,
    pub who: Who,
    pub text: String,
}

fn field(v: &str) -> String {
    let v: String = v
        .chars()
        .map(|c| if c.is_whitespace() { '_' } else { c })
        .collect();
    if v.is_empty() { "-".into() } else { v }
}

/// Newlines as single spaces: how every line shows a text.
pub fn flat(s: &str) -> String {
    s.replace("\r\n", " ").replace(['\r', '\n'], " ")
}

fn valid_ts(ts: &str) -> bool {
    DateTime::strptime(TS, ts).is_ok()
}

impl Message {
    /// Unstamped: [`crate::Store::append`] gives it the time it lands in the log.
    pub fn new(kind: Kind, text: &str) -> Message {
        Message {
            ts: String::new(),
            kind,
            origin: String::new(),
            place: Place::default(),
            who: Who::default(),
            text: text.into(),
        }
    }

    pub fn encode(&self) -> Vec<u8> {
        let (p, w) = (&self.place, &self.who);
        let head = [
            self.ts.as_str(),
            self.kind.name(),
            &self.origin,
            &p.repo,
            &p.head,
            &p.branch,
            &w.agent,
            &w.model,
            &w.session,
        ]
        .map(field)
        .join(" ");
        format!("{head}\n{}", self.text).into_bytes()
    }

    pub fn decode(b: &[u8]) -> Result<Message> {
        let corrupt = || anyhow::anyhow!("A message is corrupt.");
        let s = std::str::from_utf8(b).map_err(|_| corrupt())?;
        let (head, text) = s.split_once('\n').ok_or_else(corrupt)?;
        let f: Vec<&str> = head.split(' ').collect();
        let [ts, kind, origin, repo, hd, branch, agent, model, session] = f[..] else {
            return Err(corrupt());
        };
        if !valid_ts(ts) {
            return Err(corrupt());
        }
        Ok(Message {
            ts: ts.into(),
            kind: Kind::parse(kind).ok_or_else(corrupt)?,
            origin: origin.into(),
            place: Place {
                repo: repo.into(),
                head: hd.into(),
                branch: branch.into(),
            },
            who: Who {
                agent: agent.into(),
                model: model.into(),
                session: session.into(),
            },
            text: text.into(),
        })
    }

    /// The message as the tree sees it: a note bare, a chat message as `kind: text`.
    pub fn label(&self) -> String {
        match self.kind {
            Kind::Note => self.text.clone(),
            k => format!("{}: {}", k.name(), self.text),
        }
    }

    /// Its leaf: the flat label, or empty for a kind not [`Kind::shown`].
    pub fn line(&self) -> String {
        if self.kind.shown() { flat(&self.label()) } else { String::new() }
    }

    /// `YYYY-MM-DD HH:MM` in the time zone `tz`.
    pub fn time_in(&self, tz: &TimeZone) -> String {
        let at = DateTime::strptime(TS, &self.ts)
            .and_then(|dt| dt.to_zoned(TimeZone::UTC))
            .expect("a decoded timestamp");
        at.with_time_zone(tz.clone())
            .strftime("%Y-%m-%d %H:%M")
            .to_string()
    }
}

impl Node {
    pub fn encode(&self) -> String {
        let w = &self.who;
        let head = [&self.ts, &self.origin, &w.agent, &w.model, &w.session].map(|f| field(f));
        format!("{} {}", head.join(" "), flat(&self.text))
    }

    pub fn decode(line: &str) -> Option<Node> {
        let mut f = line.splitn(6, ' ');
        let [ts, origin, agent, model, session, text] = [(); 6].map(|_| f.next().unwrap_or(""));
        valid_ts(ts).then(|| Node {
            ts: ts.into(),
            origin: origin.into(),
            who: Who {
                agent: agent.into(),
                model: model.into(),
                session: session.into(),
            },
            text: text.into(),
        })
    }
}

pub fn now() -> String {
    ts(jiff::Timestamp::now().to_zoned(TimeZone::UTC).datetime())
}

fn ts(dt: DateTime) -> String {
    dt.strftime(TS).to_string()
}

pub fn midnight(date: jiff::civil::Date) -> String {
    ts(date.to_datetime(jiff::civil::Time::midnight()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn message() -> Message {
        Message {
            ts: "20261003T142233Z".into(),
            kind: Kind::Tool,
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
            text: "line one\nline two\n".into(),
        }
    }

    #[test]
    fn messages_round_trip() {
        let m = message();
        let b = m.encode();
        assert!(
            b.starts_with(b"20261003T142233Z tool 7KQ2ZD acme/widget 0123456789ab feature/a_b ")
        );
        let back = Message::decode(&b).unwrap();
        assert_eq!(back.origin, "7KQ2ZD");
        assert_eq!(back.place.branch, "feature/a_b");
        assert_eq!(back.who.model, "-");
        assert_eq!(back.who.session, "ünï");
        assert_eq!(back.text, m.text);
        assert_eq!(back.label(), "tool: line one\nline two\n");
        let note = Message { kind: Kind::Note, ..back.clone() };
        assert_eq!(note.label(), "line one\nline two\n");
        let tz = TimeZone::fixed(jiff::tz::offset(-15));
        assert_eq!(back.time_in(&tz), "2026-10-02 23:22");
        assert_eq!(back.time_in(&TimeZone::UTC), "2026-10-03 14:22");
        assert_eq!(Message::decode(&back.encode()).unwrap(), back);
        assert!(Message::decode(b"garbage").is_err());
        assert!(Message::decode(b"20261003T142233Z chat - - - - - - -\nx").is_err());
        assert!(Message::decode(b"20261003T142233Z note - - - - - -\nx").is_err());
        assert!(Message::decode(&[0xff, b'\n']).is_err());
    }

    #[test]
    fn nodes_are_one_line() {
        let n = Node {
            ts: "20261003T142233Z".into(),
            origin: "7KQ2ZD".into(),
            who: Who {
                agent: "claude-code".into(),
                model: "".into(),
                session: "s 1".into(),
            },
            text: "a\nb\r\nc".into(),
        };
        let line = n.encode();
        assert_eq!(line, "20261003T142233Z 7KQ2ZD claude-code - s_1 a b c");
        let back = Node::decode(&line).unwrap();
        assert_eq!(back.origin, "7KQ2ZD");
        assert_eq!(
            (back.who.model.as_str(), back.who.session.as_str()),
            ("-", "s_1")
        );
        assert_eq!(back.text, "a b c");
        assert_eq!(Node::decode(&back.encode()), Some(back));
        assert!(Node::decode("").is_none());
        assert_eq!(Node::decode("20261003T142233Z O a m s ").unwrap().text, "");
        assert_eq!(Kind::parse("note"), Some(Kind::Note));
        assert_eq!(Kind::parse("work"), None);
    }
}
