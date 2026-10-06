use std::{
    collections::VecDeque,
    io::Write,
    process::Command,
    sync::mpsc::{Receiver, RecvTimeoutError, Sender},
    time::{Duration, Instant},
};

use ai_memory::{
    Block, Compactor, Kind, Message, Place, Who,
    claude::{Live, Proc},
};
use anyhow::Result;
use serde_json::Value;

use crate::{
    master::Master,
    prime::{Primed, Primer, blocks},
    stream::{Act, Mapper},
};

/// How long the view must stay unchanged, with nothing running, before it is primed.
pub const IDLE: Duration = Duration::from_secs(1);

pub enum Event {
    /// One message from the user.
    Input(String),
    /// Ctrl-C: stop the turn, or the wait before it.
    Cancel,
    /// Ctrl-D or a signal: stop everything and leave.
    Exit,
    /// The input ended: finish the work queued, then leave.
    End,
    /// A stdout line of call `id`; `None` at its end.
    Line(u64, Option<String>),
    /// A compactor call ended.
    Compacted,
}

/// Starts `claude` calls whose output comes back as [`Event::Line`]s.
pub struct Spawner {
    tx: Sender<Event>,
    next: u64,
    live: Live,
}

impl Spawner {
    pub fn new(tx: Sender<Event>) -> Self {
        Self {
            tx,
            next: 0,
            live: Live::default(),
        }
    }

    /// A turn ends at its first `result`, a priming at its first `message_start`: the child is
    /// killed there, before anything after it (a follow-up turn) can run.
    pub fn spawn(&mut self, cmd: Command, prime: bool) -> Result<(u64, Proc)> {
        let id = self.next;
        self.next += 1;
        let tx = self.tx.clone();
        let stop: fn(&Value) -> bool = match prime {
            true => |ev| {
                ev["type"] == "result"
                    || (ev["type"] == "stream_event" && ev["event"]["type"] == "message_start")
            },
            false => |ev| ev["type"] == "result",
        };
        let proc = Proc::spawn_until(cmd, Some(&self.live), Some(stop), move |line| {
            let _ = tx.send(Event::Line(id, line));
        })?;
        Ok((id, proc))
    }
}

impl Drop for Spawner {
    fn drop(&mut self) {
        self.live.kill_all();
    }
}

/// Plain terminal output: streamed text as it comes, status lines on lines of their own.
struct Printer<'w, W: Write> {
    out: &'w mut W,
    col0: bool,
}

/// `s` without control characters but newlines and tabs: model and tool output must not drive the terminal.
fn plain(s: &str) -> String {
    s.chars()
        .filter(|&c| c == '\n' || c == '\t' || !c.is_control())
        .collect()
}

impl<W: Write> Printer<'_, W> {
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

struct Turn {
    id: u64,
    proc: Proc,
    /// Messages sent mid-run, not yet taken.
    sent: VecDeque<String>,
    mapper: Mapper,
    cancelled: bool,
}

#[derive(Default)]
struct State {
    queue: VecDeque<String>,
    turn: Option<Turn>,
    primer: Primer,
    prime_failed: bool,
    /// The view as last seen, and since when it is unchanged if it awaits a background priming.
    seen: String,
    changed: Option<Instant>,
    waiting: bool,
    ending: bool,
    exit: bool,
}

pub struct Session<'a, 's> {
    pub compact: &'a mut Compactor<'s>,
    pub master: &'a Master,
    pub spawn: Spawner,
    pub place: Place,
    pub who: Who,
    pub prime: bool,
}

impl Session<'_, '_> {
    fn log(&mut self, kind: Kind, text: &str) -> Result<()> {
        let message = Message {
            place: self.place.clone(),
            who: self.who.clone(),
            ..Message::new(kind, text)
        };
        self.compact.log(&[message])?;
        Ok(())
    }

    fn unanswered(&mut self, texts: impl IntoIterator<Item = String>) -> Result<()> {
        for text in texts {
            self.log(Kind::User, &text)?;
        }
        Ok(())
    }

    /// The chat loop (spec §7): prints the view, then runs a fresh call per batch of messages
    /// until [`Event::Exit`], or [`Event::End`] once idle.
    pub fn run(&mut self, events: Receiver<Event>, out: &mut impl Write) -> Result<()> {
        let tx = self.spawn.tx.clone();
        self.compact.notify(move || {
            let _ = tx.send(Event::Compacted);
        });
        let mut p = Printer { out, col0: true };
        let mut st = State {
            seen: self.compact.mem().render(),
            ..State::default()
        };
        p.text(&format!("{}\n", st.seen))?;
        p.prompt()?;
        loop {
            self.compact.step(Duration::ZERO)?;
            for r in self.compact.take_reports() {
                p.info(&format!("compactor: {r}"))?;
            }
            if let Some(done) = st.primer.tick(Instant::now()) {
                self.primed(&mut st, &mut p, done)?;
            }
            self.advance(&mut st, &mut p)?;
            if st.exit || st.ending && st.turn.is_none() && st.queue.is_empty() {
                return Ok(());
            }
            p.out.flush()?;
            let deadline = [
                st.changed.map(|at| at + IDLE),
                st.primer.deadline(),
                self.compact.next_retry(),
            ]
            .into_iter()
            .flatten()
            .min();
            let first = match deadline {
                Some(at) => match events.recv_timeout(at.saturating_duration_since(Instant::now()))
                {
                    Ok(ev) => ev,
                    Err(RecvTimeoutError::Timeout) => continue,
                    Err(RecvTimeoutError::Disconnected) => Event::Exit,
                },
                None => events.recv().unwrap_or(Event::Exit),
            };
            self.handle(&mut st, &mut p, first)?;
            while let Ok(ev) = events.try_recv() {
                if st.exit {
                    break;
                }
                self.handle(&mut st, &mut p, ev)?;
            }
        }
    }

    /// Starts what is due: the priming and then the turn for queued messages once every view
    /// line is a summary, else a background priming of a view that stayed unchanged.
    fn advance<W: Write>(&mut self, st: &mut State, p: &mut Printer<W>) -> Result<()> {
        if st.turn.is_some() {
            return Ok(());
        }
        if !st.queue.is_empty() {
            self.compact.refresh()?;
            let mem = self.compact.mem();
            if !mem.all_built() {
                if !st.waiting {
                    let n = mem.view().iter().filter(|&&c| !mem.built(c)).count();
                    p.info(&format!(
                        "waiting for {} …",
                        ai_memory::plural(n as u64, "summary")
                    ))?;
                    st.waiting = true;
                }
                return Ok(());
            }
            st.waiting = false;
            let view = mem.render();
            if self.prime && !st.primer.fresh(&view) && !st.primer.failed(&view) {
                if let Err(e) = st.primer.start(self.master, &mut self.spawn, &view) {
                    self.primed(st, p, Primed::No(format!("{e:#}")))?;
                    return self.advance(st, p);
                }
                return Ok(());
            }
            return self.ask(st, p, &view);
        }
        let view = self.compact.mem().render();
        if view != st.seen {
            st.seen = view;
            st.changed = Some(Instant::now());
        }
        let due = st.changed.is_some_and(|at| at.elapsed() >= IDLE);
        if due && self.prime && st.primer.running().is_none() {
            st.changed = None;
            if self.compact.mem().all_built()
                && !st.primer.fresh(&st.seen)
                && let Err(e) = st.primer.start(self.master, &mut self.spawn, &st.seen)
            {
                self.primed(st, p, Primed::No(format!("{e:#}")))?;
            }
        }
        Ok(())
    }

    /// Logs the queued messages after rendering `view`, and sends both to a fresh call.
    fn ask<W: Write>(&mut self, st: &mut State, p: &mut Printer<W>, view: &str) -> Result<()> {
        let texts: Vec<_> = st.queue.drain(..).collect();
        self.unanswered(texts.clone())?;
        let (id, mut proc) = match self.spawn.spawn(self.master.command(false), false) {
            Ok(call) => call,
            Err(e) => return p.info(&format!("error: {e:#}")),
        };
        let mut msg: Vec<Block> = blocks(view)
            .into_iter()
            .map(|b| Block::new(b, false))
            .collect();
        msg.push(Block::new(texts.join("\n\n"), false));
        let _ = proc.send(&msg);
        st.turn = Some(Turn {
            id,
            proc,
            sent: VecDeque::new(),
            mapper: Mapper::default(),
            cancelled: false,
        });
        Ok(())
    }

    fn primed<W: Write>(&mut self, st: &mut State, p: &mut Printer<W>, done: Primed) -> Result<()> {
        match done {
            Primed::Yes(info) => {
                st.prime_failed = false;
                p.info(&info)
            }
            Primed::No(why) if !st.prime_failed => {
                st.prime_failed = true;
                p.info(&format!("priming failed: {why}; proceeding unprimed"))
            }
            Primed::No(_) => Ok(()),
        }
    }

    fn handle<W: Write>(&mut self, st: &mut State, p: &mut Printer<W>, ev: Event) -> Result<()> {
        match ev {
            Event::Input(text) if text.trim().is_empty() => {}
            Event::Input(text) => match st.turn.as_mut().filter(|t| !t.cancelled) {
                Some(t) => {
                    let _ = t.proc.send(&[Block::new(text.as_str(), false)]);
                    t.sent.push_back(text);
                }
                None => st.queue.push_back(text),
            },
            Event::Cancel => {
                if let Some(t) = st.turn.as_mut() {
                    t.cancelled = true;
                    t.proc.kill();
                    p.info("cancelled")?;
                } else if !st.queue.is_empty() {
                    self.unanswered(std::mem::take(&mut st.queue))?;
                    st.waiting = false;
                    p.info("cancelled")?;
                    p.prompt()?;
                } else {
                    p.info("Ctrl-D exits")?;
                    p.prompt()?;
                }
            }
            Event::Exit => {
                if let Some(t) = st.turn.take() {
                    t.proc.kill();
                    self.unanswered(t.sent)?;
                }
                self.unanswered(std::mem::take(&mut st.queue))?;
                st.exit = true;
            }
            Event::End => st.ending = true,
            Event::Compacted => {}
            Event::Line(id, line) => {
                let ev = line
                    .as_deref()
                    .and_then(|l| serde_json::from_str::<Value>(l).ok());
                if (line.is_none() || ev.is_some())
                    && let Some(done) = st.primer.line(id, ev.as_ref())
                {
                    self.primed(st, p, done)?;
                }
                if st.turn.as_ref().is_some_and(|t| t.id == id) {
                    match (line, ev) {
                        (None, _) => self.finish(st, p, None)?,
                        (Some(_), Some(ev)) => self.event(st, p, &ev)?,
                        (Some(_), None) => {}
                    }
                }
            }
        }
        Ok(())
    }

    /// Shows and logs one event of the running turn, in stream order.
    fn event<W: Write>(&mut self, st: &mut State, p: &mut Printer<W>, ev: &Value) -> Result<()> {
        let t = st.turn.as_mut().expect("a running turn");
        let acts = t.mapper.map(ev);
        if !t.mapper.model.is_empty() {
            self.who.model.clone_from(&t.mapper.model);
        }
        for act in acts {
            match act {
                Act::Show(text) => p.text(&text)?,
                Act::Info(text) => p.info(&text)?,
                Act::Log(kind, text) => self.log(kind, &text)?,
                Act::Taken => {
                    let taken = st.turn.as_mut().and_then(|t| t.sent.pop_front());
                    if let Some(text) = taken {
                        self.log(Kind::User, &text)?;
                    }
                }
            }
        }
        if ev["type"] == "result" {
            self.finish(st, p, Some(ev))?;
        }
        Ok(())
    }

    /// Ends the turn. After a `result`, the messages the call never took go back to the queue
    /// for a fresh call; after a cancel or a crash they are logged, unanswered.
    fn finish<W: Write>(
        &mut self,
        st: &mut State,
        p: &mut Printer<W>,
        result: Option<&Value>,
    ) -> Result<()> {
        let mut t = st.turn.take().expect("a running turn");
        t.proc.kill();
        match result {
            Some(ev) => {
                if let Err(e) = ai_memory::claude::reply(ev) {
                    p.info(&e.to_string())?;
                }
                p.info(&usage(ev))?;
            }
            None if !t.cancelled => p.info(&t.proc.failure())?,
            None => {}
        }
        match result.filter(|_| !t.cancelled) {
            Some(_) => {
                for text in t.sent.into_iter().rev() {
                    st.queue.push_front(text);
                }
            }
            None => self.unanswered(t.sent)?,
        }
        if st.queue.is_empty() {
            p.prompt()?;
        }
        Ok(())
    }
}

fn usage(ev: &Value) -> String {
    let u = &ev["usage"];
    let n = |k: &str| u[k].as_u64().unwrap_or(0);
    format!(
        "{} in · {} read · {} write · {} out · {:.1} s",
        n("input_tokens"),
        n("cache_read_input_tokens"),
        n("cache_creation_input_tokens"),
        n("output_tokens"),
        ev["duration_ms"].as_u64().unwrap_or(0) as f64 / 1000.0
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use ai_memory::{Backend, Conversation, Options, Store};
    use anyhow::bail;
    use std::os::unix::fs::PermissionsExt;
    use std::{
        fs,
        sync::{Arc, mpsc::channel},
        thread,
    };

    struct NoCalls;
    impl Backend for NoCalls {
        fn agent(&self) -> &str {
            "fake"
        }
        fn model(&self) -> &str {
            "fake"
        }
        fn open(&self, _: &str) -> Result<Box<dyn Conversation>> {
            bail!("unexpected compaction")
        }
    }
    struct Output {
        bytes: Vec<u8>,
        tx: Sender<Event>,
        results: usize,
        stop_after: usize,
    }
    impl Write for Output {
        fn write(&mut self, b: &[u8]) -> std::io::Result<usize> {
            self.bytes.extend_from_slice(b);
            if self
                .bytes
                .windows(" write · 4 out".len())
                .filter(|w| *w == " write · 4 out".as_bytes())
                .count()
                > self.results
            {
                self.results += 1;
                if self.results == self.stop_after {
                    let _ = self.tx.send(Event::Exit);
                }
            }
            Ok(b.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    /// A bash `claude` in `dir` that runs `body` there.
    fn fake(dir: &std::path::Path, body: &str) -> std::ffi::OsString {
        let bash = std::env::split_paths(&std::env::var_os("PATH").unwrap())
            .map(|d| d.join("bash"))
            .find(|p| p.is_file())
            .expect("bash on PATH");
        let src = dir.join("claude.sh");
        fs::write(
            &src,
            format!("#!{}\ncd '{}'\n{body}", bash.display(), dir.display()),
        )
        .unwrap();
        let p = dir.join("claude");
        assert!(
            Command::new("cp")
                .arg(&src)
                .arg(&p)
                .status()
                .unwrap()
                .success()
        );
        fs::set_permissions(&p, fs::Permissions::from_mode(0o755)).unwrap();
        p.into()
    }
    /// Runs `f` beside a session; if it panics, the session exits so the test fails, not hangs.
    fn drive<T: Send + 'static>(
        tx: &Sender<Event>,
        f: impl FnOnce(&Sender<Event>) -> T + Send + 'static,
    ) -> thread::JoinHandle<T> {
        struct ExitOnPanic(Sender<Event>);
        impl Drop for ExitOnPanic {
            fn drop(&mut self) {
                if thread::panicking() {
                    let _ = self.0.send(Event::Exit);
                }
            }
        }
        let guard = ExitOnPanic(tx.clone());
        thread::spawn(move || f(&guard.0))
    }
    fn wait_file(path: &std::path::Path) {
        let deadline = Instant::now() + Duration::from_secs(5);
        while !path.exists() {
            assert!(Instant::now() < deadline, "missing {}", path.display());
            thread::sleep(Duration::from_millis(5));
        }
    }
    #[test]
    fn fresh_turns_replay_taken_messages_requeue_pending_and_prime() {
        let temp = tempfile::TempDir::new().unwrap();
        let body = r#"
read -r initial
if [ "$DISABLE_PROMPT_CACHING" = 1 ]; then
  echo "$initial" >> primes
  echo '{"type":"stream_event","event":{"type":"message_start","message":{"usage":{"cache_read_input_tokens":12,"cache_creation_input_tokens":3}}}}'
  sleep 60
  exit
fi
echo "$initial" >> turns
echo '{"type":"system","subtype":"init","model":"fake-opus","mcp_servers":[{"name":"memory","status":"connected"}]}'
echo '{"type":"user","isReplay":true}'
if [ ! -f first ]; then
  touch started
  read -r steering
  echo '{"type":"user","isReplay":true}'
  read -r pending
  echo '{"type":"assistant","message":{"content":[{"type":"thinking","thinking":"SECRET"},{"type":"tool_use","name":"Read","input":{"file_path":"x"}}]}}'
  echo '{"type":"user","message":{"content":[{"type":"tool_result","content":"tool output"}]}}'
  touch first
fi
echo '{"type":"stream_event","event":{"type":"content_block_delta","delta":{"type":"text_delta","text":"reply"}}}'
echo '{"type":"assistant","message":{"content":[{"type":"text","text":"reply"}]}}'
echo '{"type":"result","result":"reply","usage":{"input_tokens":1,"cache_read_input_tokens":2,"cache_creation_input_tokens":3,"output_tokens":4}}'
sleep 60
echo '{"type":"assistant","message":{"content":[{"type":"text","text":"BAD FOLLOWUP"}]}}'
"#;
        let program = fake(temp.path(), body);
        let (store, _) = Store::create(&temp.path().join("memory")).unwrap();
        store
            .append("seed", &[Message::new(Kind::Note, "older context")])
            .unwrap();
        let mut compact = Compactor::new(&store, Arc::new(NoCalls), Options::default())
            .unwrap()
            .unwrap();
        let master = Master::new(program, "opus", "high", "constant", "{}".into()).unwrap();
        let (tx, rx) = channel();
        tx.send(Event::Input("opening".into())).unwrap();
        let started = temp.path().join("started");
        let driver = drive(&tx, move |inject| {
            wait_file(&started);
            inject.send(Event::Input("taken".into())).unwrap();
            inject.send(Event::Input("pending".into())).unwrap();
        });
        let mut out = Output {
            bytes: vec![],
            tx: tx.clone(),
            results: 0,
            stop_after: 2,
        };
        Session {
            compact: &mut compact,
            master: &master,
            spawn: Spawner::new(tx),
            place: Place::default(),
            who: Who::default(),
            prime: true,
        }
        .run(rx, &mut out)
        .unwrap();
        driver.join().unwrap();
        let snap = store.snapshot().unwrap();
        let messages: Vec<_> = (0..snap.log_len().unwrap())
            .map(|i| snap.message(i).unwrap())
            .collect();
        let users: Vec<_> = messages
            .iter()
            .filter(|m| m.kind == Kind::User)
            .map(|m| m.text.as_str())
            .collect();
        assert_eq!(users, ["opening", "taken", "pending"]);
        assert_eq!(messages.iter().filter(|m| m.kind == Kind::Talk).count(), 2);
        assert!(
            messages
                .iter()
                .any(|m| m.kind == Kind::Tool && m.text == "Read {\"file_path\":\"x\"}")
        );
        assert!(
            messages
                .iter()
                .any(|m| m.kind == Kind::Echo && m.text == "tool output")
        );
        assert!(
            messages
                .iter()
                .all(|m| !m.text.contains("SECRET") && !m.text.contains("BAD FOLLOWUP"))
        );
        let turns: Vec<Value> = fs::read_to_string(temp.path().join("turns"))
            .unwrap()
            .lines()
            .map(|l| serde_json::from_str(l).unwrap())
            .collect();
        assert_eq!(turns.len(), 2);
        assert!(
            turns[0]["message"]["content"][0]["text"]
                .as_str()
                .unwrap()
                .contains("older context")
        );
        assert!(
            !turns[0]["message"]["content"][0]["text"]
                .as_str()
                .unwrap()
                .contains("opening")
        );
        assert_eq!(turns[1]["message"]["content"][1]["text"], "pending");
        assert!(
            turns[1]["message"]["content"][0]["text"]
                .as_str()
                .unwrap()
                .contains("taken")
        );
        let primes: Vec<Value> = fs::read_to_string(temp.path().join("primes"))
            .unwrap()
            .lines()
            .map(|l| serde_json::from_str(l).unwrap())
            .collect();
        for (prime, turn) in primes.iter().zip(&turns) {
            assert_eq!(
                prime["message"]["content"][0]["text"],
                turn["message"]["content"][0]["text"]
            );
            assert_eq!(
                prime["message"]["content"][0]["cache_control"]["type"],
                "ephemeral"
            );
            assert!(turn["message"]["content"][0].get("cache_control").is_none());
        }
    }

    #[test]
    fn cancellation_while_settling_preserves_user_without_starting_turn() {
        let temp = tempfile::TempDir::new().unwrap();
        let (store, _) = Store::create(&temp.path().join("memory")).unwrap();
        store
            .append("seed", &[Message::new(Kind::Note, &"long ".repeat(150))])
            .unwrap();
        let mut compact = Compactor::new(&store, Arc::new(NoCalls), Options::default())
            .unwrap()
            .unwrap();
        let program = fake(temp.path(), "touch called\nsleep 60\n");
        let master = Master::new(program, "opus", "high", "constant", "{}".into()).unwrap();
        let (tx, rx) = channel();
        tx.send(Event::Input("unanswered".into())).unwrap();
        tx.send(Event::Cancel).unwrap();
        tx.send(Event::Exit).unwrap();
        Session {
            compact: &mut compact,
            master: &master,
            spawn: Spawner::new(tx),
            place: Place::default(),
            who: Who::default(),
            prime: false,
        }
        .run(rx, &mut vec![])
        .unwrap();
        assert_eq!(
            store.snapshot().unwrap().message(1).unwrap().text,
            "unanswered"
        );
        assert!(!temp.path().join("called").exists());
    }

    #[test]
    fn prime_failure_proceeds_with_a_real_turn_and_reports_usage() {
        let temp = tempfile::TempDir::new().unwrap();
        let program = fake(
            temp.path(),
            r#"
read -r initial
if [ "$DISABLE_PROMPT_CACHING" = 1 ]; then
echo '{"type":"result","is_error":true,"result":"prime denied"}'
else
echo '{"type":"assistant","message":{"content":[{"type":"text","text":"completed"}]}}'
echo '{"type":"result","result":"completed","usage":{"input_tokens":1,"cache_read_input_tokens":2,"cache_creation_input_tokens":3,"output_tokens":4}}'
fi
sleep 60
"#,
        );
        let (store, _) = Store::create(&temp.path().join("memory")).unwrap();
        let mut compact = Compactor::new(&store, Arc::new(NoCalls), Options::default())
            .unwrap()
            .unwrap();
        let master = Master::new(program, "opus", "high", "constant", "{}".into()).unwrap();
        let (tx, rx) = channel();
        tx.send(Event::Input("question".into())).unwrap();
        let mut out = Output {
            bytes: vec![],
            tx: tx.clone(),
            results: 0,
            stop_after: 1,
        };
        Session {
            compact: &mut compact,
            master: &master,
            spawn: Spawner::new(tx),
            place: Place::default(),
            who: Who::default(),
            prime: true,
        }
        .run(rx, &mut out)
        .unwrap();
        let output = String::from_utf8(out.bytes).unwrap();
        assert!(output.contains("priming failed: claude: prime denied; proceeding unprimed"));
        assert!(output.contains("1 in · 2 read · 3 write · 4 out"));
        assert_eq!(
            store.snapshot().unwrap().message(1).unwrap().text,
            "completed"
        );
    }

    #[test]
    fn an_external_unsummarized_note_during_priming_blocks_the_turn() {
        let temp = tempfile::TempDir::new().unwrap();
        let program = fake(
            temp.path(),
            r#"
read -r initial
if [ "$DISABLE_PROMPT_CACHING" = 1 ]; then
touch started
while [ ! -f appended ]; do sleep .01; done
echo '{"type":"stream_event","event":{"type":"message_start"}}'
else
touch master_called
fi
sleep 60
"#,
        );
        let memdir = temp.path().join("memory");
        let (store, _) = Store::create(&memdir).unwrap();
        let mut compact = Compactor::new(&store, Arc::new(NoCalls), Options::default())
            .unwrap()
            .unwrap();
        let master = Master::new(program, "opus", "high", "constant", "{}".into()).unwrap();
        let (tx, rx) = channel();
        tx.send(Event::Input("question".into())).unwrap();
        let started = temp.path().join("started");
        let appended = temp.path().join("appended");
        let driver = drive(&tx, move |inject| {
            wait_file(&started);
            Store::open(&memdir)
                .unwrap()
                .append(
                    "external",
                    &[Message::new(Kind::Note, &"long ".repeat(150))],
                )
                .unwrap();
            fs::write(appended, "").unwrap();
            thread::sleep(Duration::from_millis(100));
            inject.send(Event::Exit).unwrap();
        });
        Session {
            compact: &mut compact,
            master: &master,
            spawn: Spawner::new(tx),
            place: Place::default(),
            who: Who::default(),
            prime: true,
        }
        .run(rx, &mut vec![])
        .unwrap();
        driver.join().unwrap();
        assert!(!temp.path().join("master_called").exists());
        assert_eq!(
            store.snapshot().unwrap().message(1).unwrap().text,
            "question"
        );
    }

    #[test]
    fn unread_stdin_never_blocks_eof_for_large_opening_or_midrun_messages() {
        for initial_large in [true, false] {
            let temp = tempfile::TempDir::new().unwrap();
            let (store, _) = Store::create(&temp.path().join("memory")).unwrap();
            let program = fake(temp.path(), "touch started\ntrap '' TERM\nsleep 60\n");
            let master = Master::new(program, "opus", "high", "constant", "{}".into()).unwrap();
            let mut compact = Compactor::new(&store, Arc::new(NoCalls), Options::default())
                .unwrap()
                .unwrap();
            let (tx, rx) = channel();
            let large = "x".repeat(1_000_000);
            tx.send(Event::Input(if initial_large {
                large.clone()
            } else {
                "opening".into()
            }))
            .unwrap();
            let started = temp.path().join("started");
            let driver = drive(&tx, move |inject| {
                wait_file(&started);
                if !initial_large {
                    inject.send(Event::Input(large)).unwrap();
                }
                thread::sleep(Duration::from_millis(50));
                inject.send(Event::Exit).unwrap();
            });
            let at = Instant::now();
            Session {
                compact: &mut compact,
                master: &master,
                spawn: Spawner::new(tx),
                place: Place::default(),
                who: Who::default(),
                prime: false,
            }
            .run(rx, &mut vec![])
            .unwrap();
            driver.join().unwrap();
            assert!(at.elapsed() < Duration::from_secs(3));
            assert_eq!(
                store.snapshot().unwrap().log_len().unwrap(),
                if initial_large { 1 } else { 2 }
            );
        }
    }

    #[test]
    fn eof_during_running_turn_logs_unconsumed_input_and_reaps_child() {
        let temp = tempfile::TempDir::new().unwrap();
        let (store, _) = Store::create(&temp.path().join("memory")).unwrap();
        let program = fake(
            temp.path(),
            "read -r opening\ntouch started\ntrap '' TERM\nsleep 60\n",
        );
        let master = Master::new(program, "opus", "high", "constant", "{}".into()).unwrap();
        let mut compact = Compactor::new(&store, Arc::new(NoCalls), Options::default())
            .unwrap()
            .unwrap();
        let (tx, rx) = channel();
        tx.send(Event::Input("opening".into())).unwrap();
        let started = temp.path().join("started");
        let driver = drive(&tx, move |inject| {
            wait_file(&started);
            inject.send(Event::Input("untaken".into())).unwrap();
            inject.send(Event::Exit).unwrap();
        });
        let at = Instant::now();
        Session {
            compact: &mut compact,
            master: &master,
            spawn: Spawner::new(tx),
            place: Place::default(),
            who: Who::default(),
            prime: false,
        }
        .run(rx, &mut vec![])
        .unwrap();
        driver.join().unwrap();
        assert!(at.elapsed() < Duration::from_secs(2));
        let snap = store.snapshot().unwrap();
        assert_eq!(snap.log_len().unwrap(), 2);
        assert_eq!(snap.message(1).unwrap().text, "untaken");
    }

    fn kinds(store: &Store) -> Vec<String> {
        let snap = store.snapshot().unwrap();
        (0..snap.log_len().unwrap())
            .map(|i| snap.message(i).unwrap().label())
            .collect()
    }

    fn lines(path: &std::path::Path) -> Vec<Value> {
        fs::read_to_string(path)
            .unwrap_or_default()
            .lines()
            .map(|l| serde_json::from_str(l).unwrap())
            .collect()
    }

    fn session<'a, 's>(
        compact: &'a mut Compactor<'s>,
        master: &'a Master,
        tx: Sender<Event>,
        prime: bool,
    ) -> Session<'a, 's> {
        Session {
            compact,
            master,
            spawn: Spawner::new(tx),
            place: Place::default(),
            who: Who::default(),
            prime,
        }
    }

    const ANSWER: &str = r#"echo '{"type":"assistant","message":{"content":[{"type":"text","text":"done"}]}}'
echo '{"type":"result","result":"done","usage":{"input_tokens":1,"cache_read_input_tokens":2,"cache_creation_input_tokens":3,"output_tokens":4}}'
sleep 60
"#;

    #[test]
    fn lines_that_arrive_together_are_one_turn_and_the_input_end_waits_for_it() {
        let temp = tempfile::TempDir::new().unwrap();
        let program = fake(
            temp.path(),
            &format!("read -r initial\necho \"$initial\" >> turns\n{ANSWER}"),
        );
        let (store, _) = Store::create(&temp.path().join("memory")).unwrap();
        let mut compact = Compactor::new(&store, Arc::new(NoCalls), Options::default())
            .unwrap()
            .unwrap();
        let master = Master::new(program, "opus", "high", "constant", "{}".into()).unwrap();
        let (tx, rx) = channel();
        for e in [
            Event::Input("a".into()),
            Event::Input("b".into()),
            Event::End,
        ] {
            tx.send(e).unwrap();
        }
        let mut out = vec![];
        session(&mut compact, &master, tx, false)
            .run(rx, &mut out)
            .unwrap();
        let turns = lines(&temp.path().join("turns"));
        assert_eq!(turns.len(), 1);
        let content = turns[0]["message"]["content"].as_array().unwrap();
        assert_eq!(content.last().unwrap()["text"], "a\n\nb");
        assert_eq!(content[0]["text"], "<chat>\n</chat>");
        assert_eq!(kinds(&store), ["user: a", "user: b", "talk: done"]);
        let out = String::from_utf8(out).unwrap();
        assert!(out.starts_with("<chat>\n</chat>\n> "), "{out}");
        assert!(
            out.contains("done\n1 in · 2 read · 3 write · 4 out"),
            "{out}"
        );
    }

    #[test]
    fn a_cancel_logs_what_the_call_still_reports_and_what_it_never_took() {
        let temp = tempfile::TempDir::new().unwrap();
        let program = fake(
            temp.path(),
            r#"read -r initial
trap 'echo "{\"type\":\"user\",\"message\":{\"content\":[{\"type\":\"tool_result\",\"content\":\"killed\"}]}}"; exit 143' TERM
echo '{"type":"assistant","message":{"content":[{"type":"tool_use","name":"Bash","input":{"command":"sleep 99"}}]}}'
touch started
while :; do sleep .05; done
"#,
        );
        let (store, _) = Store::create(&temp.path().join("memory")).unwrap();
        let mut compact = Compactor::new(&store, Arc::new(NoCalls), Options::default())
            .unwrap()
            .unwrap();
        let master = Master::new(program, "opus", "high", "constant", "{}".into()).unwrap();
        let (tx, rx) = channel();
        tx.send(Event::Input("go".into())).unwrap();
        let started = temp.path().join("started");
        let driver = drive(&tx, move |inject| {
            wait_file(&started);
            inject.send(Event::Input("never taken".into())).unwrap();
            inject.send(Event::Cancel).unwrap();
            inject.send(Event::End).unwrap();
        });
        let at = Instant::now();
        session(&mut compact, &master, tx, false)
            .run(rx, &mut vec![])
            .unwrap();
        driver.join().unwrap();
        assert!(at.elapsed() < Duration::from_secs(3));
        assert_eq!(
            kinds(&store),
            [
                "user: go",
                r#"tool: Bash {"command":"sleep 99"}"#,
                "echo: killed",
                "user: never taken"
            ]
        );
    }

    #[test]
    fn the_view_is_primed_once_per_change_never_at_start_and_a_fresh_priming_is_reused() {
        let temp = tempfile::TempDir::new().unwrap();
        let program = fake(
            temp.path(),
            &format!(
                r#"read -r initial
if [ "$DISABLE_PROMPT_CACHING" = 1 ]; then
  echo "$initial" >> primes
  echo '{{"type":"stream_event","event":{{"type":"message_start","message":{{"usage":{{}}}}}}}}'
  sleep 60
fi
echo "$initial" >> turns
{ANSWER}"#
            ),
        );
        let (store, _) = Store::create(&temp.path().join("memory")).unwrap();
        store
            .append("seed", &[Message::new(Kind::Note, "older")])
            .unwrap();
        let mut compact = Compactor::new(&store, Arc::new(NoCalls), Options::default())
            .unwrap()
            .unwrap();
        let master = Master::new(program, "opus", "high", "constant", "{}".into()).unwrap();
        let (tx, rx) = channel();
        let dir = temp.path().to_path_buf();
        let count = move |f: &str| lines(&dir.join(f)).len();
        let driver = drive(&tx, move |inject| {
            let until = |n: usize, f: &str| {
                let deadline = Instant::now() + Duration::from_secs(5);
                while count(f) < n {
                    assert!(Instant::now() < deadline, "{f} < {n}");
                    thread::sleep(Duration::from_millis(10));
                }
            };
            thread::sleep(IDLE + Duration::from_millis(300));
            let at_start = count("primes");
            inject.send(Event::Input("one".into())).unwrap();
            until(1, "turns");
            until(2, "primes");
            inject.send(Event::Input("two".into())).unwrap();
            until(2, "turns");
            until(3, "primes");
            thread::sleep(IDLE + Duration::from_millis(300));
            inject.send(Event::End).unwrap();
            (at_start, count("primes"))
        });
        session(&mut compact, &master, tx, true)
            .run(rx, &mut vec![])
            .unwrap();
        let (at_start, primes) = driver.join().unwrap();
        assert_eq!((at_start, primes), (0, 3));
        let p = lines(&temp.path().join("primes"));
        let t = lines(&temp.path().join("turns"));
        let texts = |m: &Value| -> Vec<Value> {
            let c = m["message"]["content"].as_array().unwrap();
            c[..c.len() - 1].iter().map(|b| b["text"].clone()).collect()
        };
        assert_eq!(
            texts(&p[0]),
            texts(&t[0]),
            "the turn sends the very blocks primed"
        );
        assert_eq!(
            texts(&p[1]),
            texts(&t[1]),
            "turn two reuses the background priming"
        );
        assert_ne!(texts(&p[1]), texts(&p[2]));
        assert!(texts(&t[1])[0].as_str().unwrap().contains("talk: done"));
    }
}
