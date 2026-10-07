use std::{
    collections::HashSet,
    fs::{File, TryLockError},
    sync::{
        Arc,
        mpsc::{Receiver, RecvTimeoutError, Sender, channel},
    },
    thread,
    time::{Duration, Instant},
};

use anyhow::{Result, bail};

use crate::{
    backend::{Backend, Block},
    record::{Message, Who, flat},
    store::{AtPath, Store},
    tree::{Coord, NODE},
    view::{Mem, VIEW},
};

pub const COMPACT: &str = include_str!("../prompts/compact.txt");
pub const SCALE: &str = include_str!("../prompts/scale.txt");
pub const JOBS: usize = 8;
pub const TRIES: usize = 5;
pub const RETRY: Duration = Duration::from_secs(10);
pub const MARKS: [usize; 3] = [50_000, 80_000, 100_000];
const LOCK: &str = "compact.lock";

/// What one compactor call builds.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Step {
    /// One message, as `kind: text`.
    Compress(String),
    /// Two adjacent lines.
    Merge(String, String),
}

#[derive(Clone, Debug)]
pub struct Job {
    pub node: Coord,
    pub step: Step,
}

/// `s` cut after the last line end before each mark (in characters); marks past the end skipped.
pub fn cut_blocks(s: &str, marks: &[usize]) -> Vec<String> {
    let mut out = Vec::new();
    let mut from = 0;
    for &m in marks {
        let Some((at, _)) = s.char_indices().nth(m) else {
            continue;
        };
        let end = s[..at].rfind('\n').map_or(0, |k| k + 1);
        if end <= from {
            continue;
        }
        out.push(s[from..end].to_string());
        from = end;
    }
    out.push(s[from..].to_string());
    out
}

fn step_text(step: &Step) -> String {
    let scale = format!("For scale, this line is exactly {NODE} bytes:\n{SCALE}\n\n");
    match step {
        Step::Compress(m) => {
            format!("{scale}Compress this message into one line, in at most {NODE} bytes:\n{m}")
        }
        Step::Merge(a, b) => format!(
            "{scale}Merge these two lines into one, in at most {NODE} bytes:\n{}\n{}",
            flat(a),
            flat(b)
        ),
    }
}

/// The user message of a call: the step alone, with no ids (the model would copy them).
pub fn message(job: &Job) -> Vec<Block> {
    vec![Block::new(step_text(&job.step), false)]
}

/// `line` cut to its first [`NODE`] bytes, never inside a character.
pub fn cut(line: &str) -> &str {
    &line[..line.floor_char_boundary(NODE)]
}

pub fn retry_text(line: &str) -> String {
    format!(
        "That line is {} bytes; the limit is {NODE}. It must end where it is cut here:\n{}| ← LIMIT",
        line.len(),
        cut(line)
    )
}

/// Builds one node: the line the model writes, retried in the same conversation while
/// over [`NODE`] bytes, up to [`TRIES`]; the shortest try wins. Returns who wrote it, and the line.
pub fn summarize(backend: &dyn Backend, job: &Job) -> Result<(Who, String)> {
    let mut conv = backend.open(COMPACT)?;
    let mut reply = conv.say(&message(job))?;
    let mut tries: Vec<String> = Vec::new();
    loop {
        let line = flat(reply.trim());
        if line.is_empty() {
            bail!("empty reply");
        }
        let done = line.len() <= NODE || tries.len() + 1 >= TRIES;
        let retry = retry_text(&line);
        tries.push(line);
        if done {
            break;
        }
        reply = conv.say(&[Block::new(retry, false)])?;
    }
    let who = Who {
        agent: backend.agent().into(),
        model: backend.model().into(),
        session: conv.session(),
    };
    let line = tries
        .into_iter()
        .min_by_key(String::len)
        .expect("one try at least");
    Ok((who, line))
}

#[derive(Clone, Copy, Debug)]
pub struct Options {
    pub jobs: usize,
    pub retry: Duration,
    pub budget: u64,
}

impl Default for Options {
    fn default() -> Options {
        Options {
            jobs: JOBS,
            retry: RETRY,
            budget: VIEW,
        }
    }
}

type Done = (Coord, Result<(Who, String)>);
type Notify = Arc<dyn Fn() + Send + Sync>;

/// The pump: builds every node whose sources exist and whose context is all summaries,
/// up to `jobs` at once, each through `backend` on its own thread. One per memory.
pub struct Compactor<'s> {
    store: &'s Store,
    backend: Arc<dyn Backend>,
    opts: Options,
    mem: Mem,
    busy: HashSet<Coord>,
    failed: HashSet<Coord>,
    waits: Vec<(Instant, Coord)>,
    reports: Vec<String>,
    built: u64,
    tx: Sender<Done>,
    rx: Receiver<Done>,
    notify: Option<Notify>,
    _lock: File,
}

impl<'s> Compactor<'s> {
    /// Takes the memory's compactor lock; `None` if another compactor holds it.
    pub fn new(store: &'s Store, backend: Arc<dyn Backend>, opts: Options) -> Result<Option<Self>> {
        let p = store.dir().join(LOCK);
        let lock = File::create(&p).at(&p)?;
        match lock.try_lock() {
            Ok(()) => {}
            Err(TryLockError::WouldBlock) => return Ok(None),
            Err(TryLockError::Error(e)) => return Err(e).at(&p),
        }
        let mem = Mem::load(&store.snapshot()?, opts.budget)?;
        let (tx, rx) = channel();
        Ok(Some(Compactor {
            store,
            backend,
            opts,
            mem,
            busy: HashSet::new(),
            failed: HashSet::new(),
            waits: Vec::new(),
            reports: Vec::new(),
            built: 0,
            tx,
            rx,
            notify: None,
            _lock: lock,
        }))
    }

    pub fn mem(&self) -> &Mem {
        &self.mem
    }

    /// Nodes built by the model so far.
    pub fn built(&self) -> u64 {
        self.built
    }

    /// The first failure of each node since it last succeeded, once.
    pub fn take_reports(&mut self) -> Vec<String> {
        std::mem::take(&mut self.reports)
    }

    pub fn busy(&self) -> usize {
        self.busy.len()
    }

    /// Calls `f` from a job's thread each time a call ends, so an event loop knows to [`Self::step`].
    pub fn notify(&mut self, f: impl Fn() + Send + Sync + 'static) {
        self.notify = Some(Arc::new(f));
    }

    /// When the earliest failed node may be tried again.
    pub fn next_retry(&self) -> Option<Instant> {
        self.waits.iter().map(|w| w.0).min()
    }

    /// Takes in what other processes wrote since, then pumps.
    pub fn refresh(&mut self) -> Result<()> {
        self.mem.absorb(&self.store.snapshot()?)?;
        self.pump()
    }

    /// Appends messages to the log and the view, then pumps.
    pub fn log(&mut self, items: &[Message]) -> Result<u64> {
        let (first, _, snap) = self.store.append("chat", items)?;
        self.mem.absorb(&snap)?;
        self.pump()?;
        Ok(first)
    }

    fn job(&self, c: Coord) -> Result<Job> {
        let step = match c.l {
            0 => Step::Compress(self.store.snapshot()?.message(c.i)?.label()),
            _ => {
                let [a, b] = c
                    .children()
                    .map(|k| self.mem.text(k).unwrap_or_default().to_string());
                Step::Merge(a, b)
            }
        };
        Ok(Job { node: c, step })
    }

    /// Starts every node that is due, up to `jobs` running.
    pub fn pump(&mut self) -> Result<()> {
        let room = self.opts.jobs.saturating_sub(self.busy.len());
        for c in self.mem.due(|c| self.busy.contains(&c), room) {
            let job = self.job(c)?;
            self.busy.insert(c);
            let (backend, tx) = (self.backend.clone(), self.tx.clone());
            let notify = self.notify.clone();
            thread::spawn(move || {
                let r = summarize(&*backend, &job);
                let _ = tx.send((job.node, r));
                if let Some(f) = notify {
                    f();
                }
            });
        }
        Ok(())
    }

    fn finish(&mut self, (c, r): Done) -> Result<()> {
        match r {
            Ok((who, text)) => {
                let (built, snap) = self.store.put_node(c, &who, &text)?;
                for (k, t) in built {
                    self.mem.set(k, t);
                }
                self.mem.absorb(&snap)?;
                self.mem.fit(self.mem.t());
                self.busy.remove(&c);
                self.failed.remove(&c);
                self.built += 1;
            }
            Err(e) => {
                if self.failed.insert(c) {
                    self.reports.push(format!("{c}: {e:#}"));
                }
                self.waits.push((Instant::now() + self.opts.retry, c));
            }
        }
        Ok(())
    }

    /// Waits up to `timeout` for a call to end or a failed node's wait to pass, takes in
    /// every call that ended, then pumps.
    pub fn step(&mut self, timeout: Duration) -> Result<()> {
        let wait = self.next_retry().map_or(timeout, |at| {
            at.saturating_duration_since(Instant::now()).min(timeout)
        });
        match self.rx.recv_timeout(wait) {
            Ok(done) => self.finish(done)?,
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => unreachable!("the compactor holds a sender"),
        }
        while let Ok(done) = self.rx.try_recv() {
            self.finish(done)?;
        }
        let now = Instant::now();
        let busy = &mut self.busy;
        self.waits.retain(|&(at, c)| {
            let waiting = at > now;
            if !waiting {
                busy.remove(&c);
            }
            waiting
        });
        self.pump()
    }

    /// Runs until every line of the view is a summary.
    pub fn settle(&mut self) -> Result<()> {
        self.pump()?;
        while !self.mem.all_built() {
            if self.busy.is_empty() {
                bail!("The view has an unbuilt line and nothing is due: rule 3 broken.");
            }
            self.step(Duration::from_secs(60))?;
        }
        Ok(())
    }

    /// Runs until no node can be built on the latest snapshot; `each` after every step.
    pub fn run(&mut self, mut each: impl FnMut(&mut Self) -> Result<()>) -> Result<()> {
        loop {
            self.refresh()?;
            if self.busy.is_empty() {
                return Ok(());
            }
            while !self.busy.is_empty() {
                self.step(Duration::from_secs(60))?;
                each(self)?;
            }
        }
    }
}

impl Drop for Compactor<'_> {
    fn drop(&mut self) {
        self.backend.stop();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prompts_are_the_spec_s() {
        assert_eq!(SCALE.len(), NODE);
        assert!(!SCALE.contains('\n'));
        assert!(COMPACT.starts_with("You write the memory of MyAI: AI agents"));
        assert!(!COMPACT.contains("OptChat"));
        assert!(
            COMPACT
                .trim_end()
                .ends_with("non-ASCII characters cost 2-4 bytes.")
        );
    }

    #[test]
    fn blocks_cut_after_line_ends_before_each_mark() {
        let s = "aaaa\nbbbb\ncccc\ndddd";
        assert_eq!(cut_blocks(s, &[7, 12]), ["aaaa\n", "bbbb\n", "cccc\ndddd"]);
        assert_eq!(cut_blocks(s, &[2, 7, 100]), ["aaaa\n", "bbbb\ncccc\ndddd"]);
        assert_eq!(cut_blocks("é\néé\n", &[3]), ["é\n", "éé\n"]);
    }

    #[test]
    fn steps_carry_no_ids_and_no_context() {
        let job = Job {
            node: Coord::new(1, 0),
            step: Step::Merge("user: hi".into(), "ai: a\nb".into()),
        };
        let m = message(&job);
        assert_eq!(m.len(), 1, "the step alone: no context");
        assert!(!m[0].cache);
        assert!(m[0].text.starts_with(&format!(
            "For scale, this line is exactly 512 bytes:\n{SCALE}\n\n"
        )));
        assert!(m[0].text.ends_with(
            "Merge these two lines into one, in at most 512 bytes:\nuser: hi\nai: a b"
        ));
        let c = step_text(&Step::Compress("tool: x\ny".into()));
        assert!(
            c.ends_with("Compress this message into one line, in at most 512 bytes:\ntool: x\ny")
        );
    }

    #[test]
    fn retries_show_the_cut() {
        let line = format!("{}ééé", "x".repeat(NODE - 1));
        assert_eq!(cut(&line).len(), NODE - 1);
        let r = retry_text(&line);
        assert!(r.starts_with(&format!(
            "That line is {} bytes; the limit is 512.",
            line.len()
        )));
        assert!(r.ends_with(&format!("{}| ← LIMIT", "x".repeat(NODE - 1))));
    }
}
