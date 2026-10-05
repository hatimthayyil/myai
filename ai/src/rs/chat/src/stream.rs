use ai_memory::Kind;
use serde_json::Value;

use crate::mcp::SERVER;

pub const CAP: usize = 30_000;

/// What one stream-json event of a turn asks for, in order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Act {
    /// Streamed reply text, shown as it comes.
    Show(String),
    /// A status line for the user, never logged.
    Info(String),
    Log(Kind, String),
    /// Claude took the oldest message sent to it mid-run.
    Taken,
}

/// `s` within [`CAP`] characters: its head and tail, with a note of what was cut.
pub fn cap(s: &str) -> String {
    let n = s.chars().count();
    if n <= CAP {
        return s.to_string();
    }
    let head: String = s.chars().take(CAP / 2).collect();
    let tail: String = s.chars().skip(n - CAP / 2).collect();
    format!("{head}\n[… {} chars cut …]\n{tail}", n - CAP)
}

/// `s` on one line, at most `n` characters.
pub fn clip(s: &str, n: usize) -> String {
    let one = s.split_whitespace().collect::<Vec<_>>().join(" ");
    match one.char_indices().nth(n) {
        Some((at, _)) => format!("{}…", &one[..at]),
        None => one,
    }
}

fn result_text(c: &Value) -> String {
    match c {
        Value::String(s) => s.clone(),
        Value::Array(parts) => parts
            .iter()
            .map(|p| match p["type"].as_str() {
                Some("text") => p["text"].as_str().unwrap_or_default().to_string(),
                t => format!("[{}]", t.unwrap_or("?")),
            })
            .collect::<Vec<_>>()
            .join("\n"),
        _ => String::new(),
    }
}

/// Maps a turn's events to acts (spec §7): replies are `talk`, tool calls `tool` (name and
/// JSON input), results `echo` (capped). Thinking is never logged.
#[derive(Default)]
pub struct Mapper {
    replays: usize,
    thought: u64,
    streamed: bool,
    pub model: String,
}

impl Mapper {
    pub fn map(&mut self, ev: &Value) -> Vec<Act> {
        let mut acts = Vec::new();
        match ev["type"].as_str().unwrap_or_default() {
            "system" if ev["subtype"] == "init" => {
                self.model = ev["model"].as_str().unwrap_or_default().into();
                let ok = ev["mcp_servers"].as_array().is_some_and(|s| {
                    s.iter()
                        .any(|s| s["name"] == SERVER && s["status"] == "connected")
                });
                if !ok {
                    acts.push(Act::Info(format!(
                        "warning: the {SERVER} MCP server is not connected: no zoom or date"
                    )));
                }
            }
            "stream_event" => {
                let d = &ev["event"]["delta"];
                if ev["event"]["type"] == "content_block_delta" {
                    match d["type"].as_str() {
                        Some("text_delta") => {
                            self.streamed = true;
                            acts.push(Act::Show(d["text"].as_str().unwrap_or_default().into()))
                        }
                        Some("thinking_delta") => {
                            if let Some(n) = d["estimated_tokens"].as_u64() {
                                self.thought = n;
                            }
                        }
                        _ => {}
                    }
                }
            }
            "assistant" => {
                for b in ev["message"]["content"].as_array().into_iter().flatten() {
                    match b["type"].as_str() {
                        Some("thinking") => {
                            acts.push(Act::Info(format!("thought for ~{} tokens", self.thought)));
                            self.thought = 0;
                        }
                        Some("text") => {
                            let t = b["text"].as_str().unwrap_or_default();
                            if !t.trim().is_empty() {
                                if !self.streamed {
                                    acts.push(Act::Show(t.into()));
                                }
                                acts.push(Act::Log(Kind::Talk, t.into()));
                            }
                        }
                        Some("tool_use") => {
                            let call = format!(
                                "{} {}",
                                b["name"].as_str().unwrap_or_default(),
                                b["input"]
                            );
                            acts.push(Act::Info(format!("→ {}", clip(&call, 160))));
                            acts.push(Act::Log(Kind::Tool, call));
                        }
                        _ => {}
                    }
                }
                self.streamed = false;
            }
            "user" if ev["isReplay"] == true => {
                self.replays += 1;
                if self.replays > 1 {
                    acts.push(Act::Taken);
                }
            }
            "user" => {
                for b in ev["message"]["content"].as_array().into_iter().flatten() {
                    if b["type"] == "tool_result" {
                        let t = result_text(&b["content"]);
                        acts.push(Act::Info(format!("← {}", clip(&t, 160))));
                        acts.push(Act::Log(Kind::Echo, cap(&t)));
                    }
                }
            }
            _ => {}
        }
        acts
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn caps_keep_head_and_tail() {
        assert_eq!(cap("short"), "short");
        let s = format!("{}{}", "é".repeat(CAP), "x".repeat(10));
        let c = cap(&s);
        assert!(c.starts_with(&"é".repeat(CAP / 2)));
        assert!(c.ends_with(&format!("{}{}", "é".repeat(CAP / 2 - 10), "x".repeat(10))));
        assert!(c.contains("\n[… 10 chars cut …]\n"));
        assert_eq!(
            c.chars().count(),
            CAP + "\n[… 10 chars cut …]\n".chars().count()
        );
        assert_eq!(clip(" a\n b  c ", 3), "a b…");
        let e = serde_json::json!({"type": "user", "message": {"content": [
            {"type": "tool_result", "content": [{"type": "text", "text": "a"}, {"type": "image"}]}
        ]}});
        assert_eq!(
            Mapper::default().map(&e)[1],
            Act::Log(Kind::Echo, "a\n[image]".into())
        );
    }

    fn replay(fixture: &str) -> Vec<Act> {
        let mut m = Mapper::default();
        fixture
            .lines()
            .flat_map(|l| m.map(&serde_json::from_str(l).unwrap()))
            .collect()
    }

    fn logs(acts: &[Act]) -> Vec<(Kind, &str)> {
        acts.iter()
            .filter_map(|a| match a {
                Act::Log(k, t) => Some((*k, t.as_str())),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn a_recorded_turn_with_zoom_date_and_bash() {
        let acts = replay(include_str!("../tests/fixtures/turn-tools.jsonl"));
        let logs = logs(&acts);
        let kinds: Vec<_> = logs.iter().map(|l| l.0).collect();
        use Kind::*;
        assert_eq!(kinds, [Tool, Echo, Tool, Echo, Tool, Echo, Talk]);
        assert_eq!(logs[0].1, r#"mcp__memory__zoom {"id":10,"n":1}"#);
        assert!(logs[1].1.starts_with("10+0|note: Fixed the retry wrapper"));
        assert_eq!(logs[3].1, "2026-10-02 15:59");
        let shown: String = acts
            .iter()
            .filter_map(|a| match a {
                Act::Show(s) => Some(s.as_str()),
                _ => None,
            })
            .collect();
        assert_eq!(shown, logs[6].1);
        assert!(
            !acts
                .iter()
                .any(|a| matches!(a, Act::Info(i) if i.starts_with("warning")))
        );
        assert!(
            !acts.contains(&Act::Taken),
            "the opening replay is not a mid-run message"
        );
    }

    #[test]
    fn a_recorded_thought_is_shown_as_its_size_and_never_logged() {
        let acts = replay(include_str!("../tests/fixtures/turn-thinking.jsonl"));
        assert_eq!(logs(&acts), [(Kind::Talk, "142")]);
        let thoughts = acts
            .iter()
            .filter(|a| matches!(a, Act::Info(i) if i.starts_with("thought for ~")))
            .count();
        assert_eq!(thoughts, 1);
        assert_eq!(clip("ab", 2), "ab");
    }
}
