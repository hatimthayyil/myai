mod plain;

use std::fmt;

use anyhow::Result;
use serde_json::Value;

pub use plain::Plain;

/// Token counts and time of one turn.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Usage {
    pub input: u64,
    pub read: u64,
    pub write: u64,
    pub output: u64,
    pub secs: f64,
}

impl Usage {
    /// The usage a `result` event reports.
    pub fn of(ev: &Value) -> Self {
        let u = &ev["usage"];
        let n = |k: &str| u[k].as_u64().unwrap_or(0);
        Self {
            input: n("input_tokens"),
            read: n("cache_read_input_tokens"),
            write: n("cache_creation_input_tokens"),
            output: n("output_tokens"),
            secs: ev["duration_ms"].as_u64().unwrap_or(0) as f64 / 1000.0,
        }
    }
}

impl fmt::Display for Usage {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{} in · {} read · {} write · {} out · {:.1} s",
            self.input, self.read, self.write, self.output, self.secs
        )
    }
}

/// What the chat tells its user, in order.
#[derive(Clone, Debug, PartialEq)]
pub enum Show {
    /// The view at start.
    View(String),
    /// Streamed reply text.
    Text(String),
    /// A thinking block of about this many tokens ended.
    Thought(u64),
    Call {
        name: String,
        input: Value,
    },
    Result {
        text: String,
        error: bool,
    },
    Info(String),
    Error(String),
    Usage(Usage),
    /// The model the turn's call reports.
    Model(String),
    Priming,
    Primed {
        read: u64,
        write: u64,
    },
    PrimeFailed(String),
    /// Compactor calls running.
    Compacting(usize),
    Compactor(String),
    /// Queued messages wait for this many summaries.
    Waiting(usize),
    /// The user's messages not yet in the conversation, oldest first.
    Pending(Vec<String>),
    /// A message entered the conversation.
    User(String),
    /// A message logged without an answer.
    Unanswered(String),
    /// A turn started.
    Turn,
    Cancelled,
    /// Nothing runs: input is awaited.
    Ready,
}

pub trait Render {
    fn show(&mut self, s: Show) -> Result<()>;
    fn flush(&mut self) -> Result<()>;
}

/// `s` without control characters but newlines and tabs: model and tool output must not drive the terminal.
pub fn plain(s: &str) -> String {
    s.chars()
        .filter(|&c| c == '\n' || c == '\t' || !c.is_control())
        .collect()
}
