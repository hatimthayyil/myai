use std::io::Write;

use anyhow::Result;

use super::{Render, Show, plain};
use crate::stream::clip;

/// Plain terminal output: streamed text as it comes, status lines on lines of their own.
pub struct Plain<W: Write> {
    out: W,
    col0: bool,
}

impl<W: Write> Plain<W> {
    pub fn new(out: W) -> Self {
        Self { out, col0: true }
    }

    fn text(&mut self, s: &str) -> Result<()> {
        let s = plain(s);
        if !s.is_empty() {
            write!(self.out, "{s}")?;
            self.col0 = s.ends_with('\n');
        }
        Ok(())
    }

    fn info(&mut self, s: &str) -> Result<()> {
        if !self.col0 {
            writeln!(self.out)?;
        }
        writeln!(self.out, "{}", plain(s))?;
        self.col0 = true;
        Ok(())
    }

    fn prompt(&mut self) -> Result<()> {
        if !self.col0 {
            writeln!(self.out)?;
        }
        write!(self.out, "> ")?;
        self.col0 = true;
        Ok(self.out.flush()?)
    }
}

impl<W: Write> Render for Plain<W> {
    fn show(&mut self, s: Show) -> Result<()> {
        match s {
            Show::View(v) => self.text(&format!("{v}\n")),
            Show::Text(t) => self.text(&t),
            Show::Thought(n) => self.info(&format!("thought for ~{n} tokens")),
            Show::Call { name, input } => {
                self.info(&format!("→ {}", clip(&format!("{name} {input}"), 160)))
            }
            Show::Result { text, .. } => self.info(&format!("← {}", clip(&text, 160))),
            Show::Info(s) | Show::Error(s) => self.info(&s),
            Show::Usage(u) => self.info(&u.to_string()),
            Show::Primed { read, write } => {
                self.info(&format!("primed: {read} read · {write} write"))
            }
            Show::PrimeFailed(why) => {
                self.info(&format!("priming failed: {why}; proceeding unprimed"))
            }
            Show::Compactor(r) => self.info(&format!("compactor: {r}")),
            Show::Waiting(n) => self.info(&format!(
                "waiting for {} …",
                ai_memory::plural(n as u64, "summary")
            )),
            Show::Cancelled => self.info("cancelled"),
            Show::Ready => self.prompt(),
            Show::Model(_)
            | Show::Priming
            | Show::Compacting(_)
            | Show::Pending(_)
            | Show::User(_)
            | Show::Unanswered(_)
            | Show::Turn => Ok(()),
        }
    }

    fn flush(&mut self) -> Result<()> {
        Ok(self.out.flush()?)
    }
}
