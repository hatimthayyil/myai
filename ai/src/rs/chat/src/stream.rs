use ai_memory::Kind;
use serde_json::Value;

use crate::{mcp::SERVER, ui::Show};

pub const CAP: usize = 30_000;

/// What one stream-json event of a turn asks for, in order.
#[derive(Clone, Debug, PartialEq)]
pub enum Act {
    /// Shown to the user, never logged.
    Show(Show),
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

/// Maps a turn's events to acts (spec §7): replies are `ai`; a tool call is one `tool` message,
/// its name and JSON input on the first line, then its result (capped), logged once the result
/// arrives. Thinking is never logged.
#[derive(Default)]
pub struct Mapper {
    replays: usize,
    calls: Vec<(String, String)>,
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
                    acts.push(Act::Show(Show::Error(format!(
                        "warning: the {SERVER} MCP server is not connected: no zoom or date"
                    ))));
                }
            }
            "stream_event" => {
                let d = &ev["event"]["delta"];
                if ev["event"]["type"] == "content_block_delta" {
                    match d["type"].as_str() {
                        Some("text_delta") => {
                            self.streamed = true;
                            acts.push(Act::Show(Show::Text(
                                d["text"].as_str().unwrap_or_default().into(),
                            )))
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
                            acts.push(Act::Show(Show::Thought(self.thought)));
                            self.thought = 0;
                        }
                        Some("text") => {
                            let t = b["text"].as_str().unwrap_or_default();
                            if !t.trim().is_empty() {
                                if !self.streamed {
                                    acts.push(Act::Show(Show::Text(t.into())));
                                }
                                acts.push(Act::Log(Kind::Ai, t.into()));
                            }
                        }
                        Some("tool_use") => {
                            let name = b["name"].as_str().unwrap_or_default();
                            acts.push(Act::Show(Show::Call {
                                name: name.into(),
                                input: b["input"].clone(),
                            }));
                            let id = b["id"].as_str().unwrap_or_default();
                            self.calls
                                .push((id.into(), format!("{name} {}", b["input"])));
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
                        let error = b["is_error"] == true;
                        let id = b["tool_use_id"].as_str().unwrap_or_default();
                        let call = match self.calls.iter().position(|c| c.0 == id) {
                            Some(k) => self.calls.remove(k).1,
                            None => "?".into(),
                        };
                        let result = if error { format!("error: {t}") } else { t.clone() };
                        acts.push(Act::Log(Kind::Tool, format!("{call}\n{}", cap(&result))));
                        acts.push(Act::Show(Show::Result { text: t, error }));
                    }
                }
            }
            _ => {}
        }
        acts
    }

    /// The `tool` messages of the calls still waiting for a result when the turn ends.
    pub fn unfinished(&mut self) -> Vec<String> {
        self.calls
            .drain(..)
            .map(|(_, call)| format!("{call}\n(no result)"))
            .collect()
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
            Mapper::default().map(&e)[0],
            Act::Log(Kind::Tool, "?\na\n[image]".into())
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
        assert_eq!(kinds, [Tool, Tool, Tool, Ai]);
        assert!(logs[0].1.starts_with(concat!(
            r#"mcp__memory__zoom {"id":10,"n":1}"#,
            "\n10+0|note: Fixed the retry wrapper"
        )));
        assert!(logs[1].1.ends_with("}\n2026-10-02 15:59"));
        let shown: String = acts
            .iter()
            .filter_map(|a| match a {
                Act::Show(Show::Text(s)) => Some(s.as_str()),
                _ => None,
            })
            .collect();
        assert_eq!(shown, logs[3].1);
        assert!(!acts.iter().any(|a| matches!(a, Act::Show(Show::Error(_)))));
        assert!(
            !acts.contains(&Act::Taken),
            "the opening replay is not a mid-run message"
        );
    }

    #[test]
    fn a_call_is_logged_with_its_own_result() {
        let mut m = Mapper::default();
        let call = |id: &str, cmd: &str| {
            serde_json::json!({"type": "assistant", "message": {"content": [
                {"type": "tool_use", "id": id, "name": "Bash", "input": {"command": cmd}}
            ]}})
        };
        m.map(&call("a", "ls"));
        m.map(&call("b", "pwd"));
        let done = serde_json::json!({"type": "user", "message": {"content": [
            {"type": "tool_result", "tool_use_id": "b", "content": "nope", "is_error": true}
        ]}});
        assert_eq!(
            logs(&m.map(&done)),
            [(Kind::Tool, "Bash {\"command\":\"pwd\"}\nerror: nope")]
        );
        assert_eq!(m.unfinished(), ["Bash {\"command\":\"ls\"}\n(no result)"]);
        assert!(m.unfinished().is_empty());
    }

    #[test]
    fn a_recorded_thought_is_shown_as_its_size_and_never_logged() {
        let acts = replay(include_str!("../tests/fixtures/turn-thinking.jsonl"));
        assert_eq!(logs(&acts), [(Kind::Ai, "142")]);
        let thoughts = acts
            .iter()
            .filter(|a| matches!(a, Act::Show(Show::Thought(_))))
            .count();
        assert_eq!(thoughts, 1);
        assert_eq!(clip("ab", 2), "ab");
    }
}
