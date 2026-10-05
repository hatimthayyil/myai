use anyhow::Result;
use jiff::{civil::DateTime, tz::TimeZone};

const TS: &str = "%Y%m%dT%H%M%SZ";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, clap::ValueEnum)]
pub enum Kind {
    User,
    Talk,
    Tool,
    Echo,
    Note,
}

impl Kind {
    pub const ALL: [Kind; 5] = [Kind::User, Kind::Talk, Kind::Tool, Kind::Echo, Kind::Note];

    pub fn name(self) -> &'static str {
        match self {
            Kind::User => "user",
            Kind::Talk => "talk",
            Kind::Tool => "tool",
            Kind::Echo => "echo",
            Kind::Note => "note",
        }
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

fn date(ts: &str) -> String {
    format!("{}-{}-{}", &ts[..4], &ts[4..6], &ts[6..8])
}

fn time(ts: &str) -> String {
    format!("{}:{}", &ts[9..11], &ts[11..13])
}

fn valid_ts(ts: &str) -> bool {
    DateTime::strptime(TS, ts).is_ok()
}

impl Message {
    pub fn new(kind: Kind, text: &str) -> Message {
        Message {
            ts: now(),
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

    /// `kind: text`, the message as the tree sees it.
    pub fn label(&self) -> String {
        format!("{}: {}", self.kind.name(), self.text)
    }

    pub fn date(&self) -> String {
        date(&self.ts)
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

    /// `<date> <hh:mm> <origin> <repo>`, UTC.
    pub fn stamp(&self) -> String {
        format!(
            "{} {} {} {}",
            date(&self.ts),
            time(&self.ts),
            self.origin,
            self.place.repo
        )
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
        (valid_ts(ts) && !text.is_empty()).then(|| Node {
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

    pub fn date(&self) -> String {
        format!("{} {}", date(&self.ts), time(&self.ts))
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
            kind: Kind::Echo,
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
            b.starts_with(b"20261003T142233Z echo 7KQ2ZD acme/widget 0123456789ab feature/a_b ")
        );
        let back = Message::decode(&b).unwrap();
        assert_eq!(back.origin, "7KQ2ZD");
        assert_eq!(back.place.branch, "feature/a_b");
        assert_eq!(back.who.model, "-");
        assert_eq!(back.who.session, "ünï");
        assert_eq!(back.text, m.text);
        assert_eq!(back.label(), "echo: line one\nline two\n");
        assert_eq!(back.stamp(), "2026-10-03 14:22 7KQ2ZD acme/widget");
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
        assert!(Node::decode("20261003T142233Z O a m s").is_none());
        assert_eq!(Kind::parse("note"), Some(Kind::Note));
        assert_eq!(Kind::parse("work"), None);
    }
}
