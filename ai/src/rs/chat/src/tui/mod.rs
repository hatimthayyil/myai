mod app;
mod cells;
mod composer;
mod markdown;
mod stream;
mod term;
mod wrap;

use std::{
    io::{self, BufWriter, Stdout, Write},
    panic,
    sync::{
        Arc, Once,
        atomic::{AtomicBool, Ordering},
        mpsc::{Receiver, RecvTimeoutError, Sender, channel},
    },
    thread,
    time::{Duration, Instant},
};

use anyhow::{Result, anyhow};
use ratatui::{
    backend::CrosstermBackend,
    crossterm::{
        cursor::Show as ShowCursor,
        event::{
            self, DisableBracketedPaste, EnableBracketedPaste, Event as Input,
            KeyboardEnhancementFlags, PopKeyboardEnhancementFlags, PushKeyboardEnhancementFlags,
        },
        execute, queue,
        terminal::{
            BeginSynchronizedUpdate, EndSynchronizedUpdate, disable_raw_mode, enable_raw_mode,
            supports_keyboard_enhancement,
        },
    },
};

use crate::{
    session::Event,
    ui::{Render, Show},
};
use app::App;
use term::Term;

type Backend = CrosstermBackend<BufWriter<Stdout>>;

enum Msg {
    Show(Show),
    Input(Input),
    Hangup,
    Quit,
}

const POLL: i32 = 100;
const STOP: Duration = Duration::from_secs(2);

static ACTIVE: AtomicBool = AtomicBool::new(false);
static ENHANCED: AtomicBool = AtomicBool::new(false);

fn setup() -> io::Result<()> {
    enable_raw_mode()?;
    ACTIVE.store(true, Ordering::SeqCst);
    execute!(io::stdout(), EnableBracketedPaste)?;
    if supports_keyboard_enhancement().unwrap_or(false) {
        execute!(
            io::stdout(),
            PushKeyboardEnhancementFlags(KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES)
        )?;
        ENHANCED.store(true, Ordering::SeqCst);
    }
    Ok(())
}

/// Gives the terminal back as it was; safe to call from any exit path, more than once.
fn restore() {
    if !ACTIVE.swap(false, Ordering::SeqCst) {
        return;
    }
    let mut out = io::stdout();
    if ENHANCED.swap(false, Ordering::SeqCst) {
        let _ = execute!(out, PopKeyboardEnhancementFlags);
    }
    let _ = execute!(out, DisableBracketedPaste, ShowCursor);
    let _ = disable_raw_mode();
}

/// The full-screen-quality terminal UI: a thread that owns the terminal, turns keys into
/// [`Event`]s and shows what the session renders.
pub struct Tui {
    tx: Sender<Msg>,
    done: Option<Receiver<io::Result<()>>>,
}

impl Tui {
    pub fn start(events: Sender<Event>, model: &str, effort: &str) -> Result<Self> {
        static HOOK: Once = Once::new();
        HOOK.call_once(|| {
            let prev = panic::take_hook();
            panic::set_hook(Box::new(move |info| {
                restore();
                prev(info);
            }));
        });
        setup()?;
        let term = match Term::new(CrosstermBackend::new(BufWriter::new(io::stdout()))) {
            Ok(t) => t,
            Err(e) => {
                restore();
                return Err(e.into());
            }
        };
        let (tx, rx) = channel();
        let stop = Arc::new(AtomicBool::new(false));
        {
            let (tx, stop) = (tx.clone(), stop.clone());
            thread::spawn(move || read(&tx, &stop));
        }
        let app = App::new(model, effort);
        let (done_tx, done) = channel();
        thread::spawn(move || {
            let result = run(term, app, &rx, &events);
            stop.store(true, Ordering::SeqCst);
            restore();
            if result.is_err() {
                let _ = events.send(Event::Exit);
            }
            let _ = done_tx.send(result);
        });
        Ok(Self {
            tx,
            done: Some(done),
        })
    }

    /// Shows what is left, then gives the terminal back.
    pub fn finish(mut self) -> Result<()> {
        self.stop()
    }

    /// Waits a bounded time for the UI thread: a terminal that blocks writes must not keep the
    /// chat alive.
    fn stop(&mut self) -> Result<()> {
        let Some(done) = self.done.take() else {
            return Ok(());
        };
        let _ = self.tx.send(Msg::Quit);
        let result = match done.recv_timeout(STOP) {
            Ok(r) => Ok(r?),
            Err(RecvTimeoutError::Timeout) => Err(anyhow!("the terminal UI did not stop")),
            Err(RecvTimeoutError::Disconnected) => Err(anyhow!("the terminal UI panicked")),
        };
        restore();
        result
    }
}

/// How the terminal on stdin is.
#[derive(PartialEq, Eq)]
enum Tty {
    Ready,
    Idle,
    Gone,
}

/// Waits up to `ms` for input on stdin. crossterm's reader spins forever on a hung-up
/// terminal, so it is only asked after this says the terminal is still there.
fn tty(ms: i32) -> Tty {
    let mut fd = libc::pollfd {
        fd: libc::STDIN_FILENO,
        events: libc::POLLIN,
        revents: 0,
    };
    match unsafe { libc::poll(&mut fd, 1, ms) } {
        n if n < 0 && io::Error::last_os_error().kind() == io::ErrorKind::Interrupted => Tty::Idle,
        n if n < 0 => Tty::Gone,
        0 => Tty::Idle,
        _ if fd.revents & (libc::POLLHUP | libc::POLLERR | libc::POLLNVAL) != 0 => Tty::Gone,
        _ => Tty::Ready,
    }
}

/// Forwards terminal events until stopped; on hangup or any error it reports the hangup and
/// ends.
fn read(tx: &Sender<Msg>, stop: &AtomicBool) {
    while !stop.load(Ordering::SeqCst) {
        if tty(POLL) == Tty::Gone {
            break;
        }
        loop {
            if tty(0) == Tty::Gone {
                let _ = tx.send(Msg::Hangup);
                return;
            }
            match event::poll(Duration::ZERO).and_then(|ready| match ready {
                true => event::read().map(Some),
                false => Ok(None),
            }) {
                Ok(Some(ev)) => {
                    if tx.send(Msg::Input(ev)).is_err() {
                        return;
                    }
                }
                Ok(None) => break,
                Err(_) => {
                    let _ = tx.send(Msg::Hangup);
                    return;
                }
            }
        }
    }
    let _ = tx.send(Msg::Hangup);
}

fn hangup() -> io::Error {
    io::Error::new(io::ErrorKind::BrokenPipe, "the terminal closed")
}

impl Drop for Tui {
    fn drop(&mut self) {
        let _ = self.stop();
    }
}

impl Render for Tui {
    fn show(&mut self, s: Show) -> Result<()> {
        self.tx
            .send(Msg::Show(s))
            .map_err(|_| anyhow!("the terminal UI stopped"))
    }

    fn flush(&mut self) -> Result<()> {
        Ok(())
    }
}

fn run(
    mut term: Term<Backend>,
    mut app: App,
    rx: &Receiver<Msg>,
    events: &Sender<Event>,
) -> io::Result<()> {
    let mut quit = false;
    while !quit {
        let tick = match app.working() {
            true => Duration::from_millis(250),
            false => Duration::from_secs(1),
        };
        let first = match rx.recv_timeout(tick) {
            Ok(m) => Some(m),
            Err(RecvTimeoutError::Timeout) => None,
            Err(RecvTimeoutError::Disconnected) => Some(Msg::Quit),
        };
        for msg in first
            .into_iter()
            .chain(std::iter::from_fn(|| rx.try_recv().ok()))
        {
            match msg {
                Msg::Show(s) => app.show(s, Instant::now()),
                Msg::Input(Input::Key(k)) => {
                    if let Some(ev) = app.key(k) {
                        let _ = events.send(ev);
                    }
                }
                Msg::Input(Input::Paste(s)) => app.composer.paste(&s),
                Msg::Input(_) => {}
                Msg::Hangup => return Err(hangup()),
                Msg::Quit => {
                    app.end();
                    quit = true;
                    break;
                }
            }
        }
        frame(&mut term, &mut app, quit)?;
    }
    term.finish()
}

/// Inserts finished lines and redraws the pane in one synchronized update.
fn frame(term: &mut Term<Backend>, app: &mut App, last: bool) -> io::Result<()> {
    if tty(0) == Tty::Gone {
        return Err(hangup());
    }
    term.autoresize()?;
    queue!(term.backend_mut(), BeginSynchronizedUpdate)?;
    term.insert(&app.take())?;
    if !last {
        let (w, h) = (term.width(), term.height());
        term.fit(app.height(w, h))?;
        let now = Instant::now();
        term.draw(|area, buf| app.render(area, buf, h, now))?;
    }
    queue!(term.backend_mut(), EndSynchronizedUpdate)?;
    term.backend_mut().flush()
}
