use std::time::{Duration, Instant};

use ai_memory::{Block, MARKS, claude::Proc, cut_blocks};
use anyhow::Result;
use serde_json::Value;

use crate::{master::Master, session::Spawner};

pub const MAX_AGE: Duration = Duration::from_secs(270);
pub const TIMEOUT: Duration = Duration::from_secs(30);

/// The view as a turn sends it: cut after the last line end before each of [`MARKS`].
pub fn blocks(view: &str) -> Vec<String> {
    cut_blocks(view, &MARKS)
}

/// How a priming call ended.
#[derive(Debug, PartialEq, Eq)]
pub enum Primed {
    /// The API took the request: the view is in the cache, with these token counts.
    Yes {
        read: u64,
        write: u64,
    },
    No(String),
}

struct Run {
    id: u64,
    proc: Proc,
    view: String,
    deadline: Instant,
}

/// Writes the view into the prompt cache: the master's own call with every block of the
/// view marked, killed at its first `message_start` (cache note, §Priming).
#[derive(Default)]
pub struct Primer {
    last: Option<(String, Instant)>,
    failed: Option<String>,
    run: Option<Run>,
}

impl Primer {
    /// Whether `view` was primed less than [`MAX_AGE`] ago.
    pub fn fresh(&self, view: &str) -> bool {
        self.last
            .as_ref()
            .is_some_and(|(v, at)| v == view && at.elapsed() < MAX_AGE)
    }

    /// Whether the last priming of `view` failed: a turn goes on without it.
    pub fn failed(&self, view: &str) -> bool {
        self.failed.as_deref() == Some(view)
    }

    pub fn running(&self) -> Option<&str> {
        self.run.as_ref().map(|r| r.view.as_str())
    }

    pub fn deadline(&self) -> Option<Instant> {
        self.run.as_ref().map(|r| r.deadline)
    }

    /// Primes `view`, ending any priming of another view.
    pub fn start(&mut self, master: &Master, spawn: &mut Spawner, view: &str) -> Result<()> {
        if self.running() == Some(view) {
            return Ok(());
        }
        self.run = None;
        self.failed = None;
        let (id, mut proc) = match spawn.spawn(master.command(true), true) {
            Ok(p) => p,
            Err(e) => {
                self.failed = Some(view.into());
                return Err(e);
            }
        };
        let mut msg: Vec<Block> = blocks(view)
            .into_iter()
            .map(|b| Block::new(b, true))
            .collect();
        msg.push(Block::new("ok", false));
        let _ = proc.send(&msg);
        self.run = Some(Run {
            id,
            proc,
            view: view.into(),
            deadline: Instant::now() + TIMEOUT,
        });
        Ok(())
    }

    /// Takes one stdout line of call `id` (`None`: its end); `Some` once the priming is over.
    pub fn line(&mut self, id: u64, line: Option<&Value>) -> Option<Primed> {
        let run = self.run.as_mut().filter(|r| r.id == id)?;
        let out = match line {
            Some(ev) if ev["type"] == "stream_event" && ev["event"]["type"] == "message_start" => {
                let u = &ev["event"]["message"]["usage"];
                Primed::Yes {
                    read: u["cache_read_input_tokens"].as_u64().unwrap_or(0),
                    write: u["cache_creation_input_tokens"].as_u64().unwrap_or(0),
                }
            }
            Some(ev) if ev["type"] == "result" => Primed::No(
                ai_memory::claude::reply(ev)
                    .err()
                    .map_or("a result before any response".into(), |e| e.to_string()),
            ),
            Some(_) => return None,
            None => Primed::No(run.proc.failure()),
        };
        Some(self.end(out))
    }

    /// Gives up on a priming past its deadline.
    pub fn tick(&mut self, now: Instant) -> Option<Primed> {
        let late = self.run.as_ref()?.deadline <= now;
        late.then(|| {
            self.end(Primed::No(format!(
                "no response after {} s",
                TIMEOUT.as_secs()
            )))
        })
    }

    fn end(&mut self, out: Primed) -> Primed {
        let run = self.run.take().expect("a priming runs");
        match out {
            Primed::Yes { .. } => self.last = Some((run.view, Instant::now())),
            Primed::No(_) => self.failed = Some(run.view),
        }
        out
    }
}
