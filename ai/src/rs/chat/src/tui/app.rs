use std::time::{Duration, Instant};

use ratatui::{
    buffer::Buffer,
    crossterm::event::{KeyEvent, KeyEventKind},
    layout::{Position, Rect},
    style::Style,
    text::{Line, Span},
    widgets::Widget,
};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use super::{
    cells,
    composer::{Action, Composer},
    stream::Stream,
    wrap,
};
use crate::{
    session::Event,
    ui::{Show, Usage, plain},
};

const LIVE_ROWS: usize = 3;
const PENDING_ROWS: usize = 3;
const SPINNER: [&str; 4] = ["◐", "◓", "◑", "◒"];

#[derive(Clone, Copy, Debug, PartialEq)]
enum Prime {
    None,
    Running,
    Done(Instant),
    Failed,
}

/// The chat as the terminal shows it: lines bound for history, and the bottom pane.
pub struct App {
    pub composer: Composer,
    model: String,
    effort: String,
    working: Option<Instant>,
    waiting: Option<usize>,
    prime: Prime,
    compacting: usize,
    usage: Option<Usage>,
    pending: Vec<String>,
    stream: Stream,
    out: Vec<Line<'static>>,
}

impl App {
    pub fn new(model: &str, effort: &str) -> Self {
        Self {
            composer: Composer::default(),
            model: model.into(),
            effort: effort.into(),
            working: None,
            waiting: None,
            prime: Prime::None,
            compacting: 0,
            usage: None,
            pending: Vec::new(),
            stream: Stream::default(),
            out: Vec::new(),
        }
    }

    /// The lines finished since last asked, for history.
    pub fn take(&mut self) -> Vec<Line<'static>> {
        std::mem::take(&mut self.out)
    }

    /// Ends the reply being streamed.
    pub fn end(&mut self) {
        let rest = self.stream.finish();
        self.out.extend(rest);
    }

    pub fn working(&self) -> bool {
        self.working.is_some()
    }

    pub fn show(&mut self, s: Show, now: Instant) {
        match s {
            Show::Text(t) => {
                let lines = self.stream.push(&t);
                self.out.extend(lines);
            }
            Show::Model(m) => self.model = m,
            Show::Priming => self.prime = Prime::Running,
            Show::Primed { .. } => self.prime = Prime::Done(now),
            Show::PrimeFailed(why) => {
                self.prime = Prime::Failed;
                self.out.extend(cells::warn(&format!(
                    "priming failed: {why}; proceeding unprimed"
                )));
            }
            Show::Compacting(n) => self.compacting = n,
            Show::Compactor(r) => self.out.extend(cells::warn(&format!("compactor: {r}"))),
            Show::Waiting(n) => self.waiting = Some(n),
            Show::Pending(p) => self.pending = p,
            s => {
                self.end();
                match s {
                    Show::View(v) => self.out.extend(cells::view(&v)),
                    Show::Thought(n) => self.out.extend(cells::thought(n)),
                    Show::Call { name, input } => self.out.extend(cells::call(&name, &input)),
                    Show::Result { text, error } => self.out.extend(cells::result(&text, error)),
                    Show::Info(s) => self.out.extend(cells::info(&s)),
                    Show::Error(s) => self.out.extend(cells::error(&s)),
                    Show::Usage(u) => {
                        self.out.extend(cells::info(&u.to_string()));
                        self.usage = Some(u);
                    }
                    Show::User(t) => self.out.extend(cells::user(&t)),
                    Show::Unanswered(t) => self.out.extend(cells::unanswered(&t)),
                    Show::Turn => {
                        self.working = Some(now);
                        self.waiting = None;
                    }
                    Show::Cancelled => {
                        self.out.extend(cells::warn("cancelled"));
                        self.waiting = None;
                    }
                    Show::Ready => {
                        self.working = None;
                        self.waiting = None;
                    }
                    _ => unreachable!(),
                }
            }
        }
    }

    pub fn key(&mut self, k: KeyEvent) -> Option<Event> {
        if k.kind == KeyEventKind::Release {
            return None;
        }
        match self.composer.key(k)? {
            Action::Submit(text) => Some(Event::Input(text)),
            Action::Cancel => Some(Event::Cancel),
            Action::Exit => Some(Event::Exit),
        }
    }

    fn live(&self, width: u16) -> Vec<Line<'static>> {
        let tail = self.stream.tail();
        if tail.is_empty() {
            return Vec::new();
        }
        let rows = wrap::line(&Line::raw(tail.replace('\t', "    ")), width as usize);
        let skip = rows.len().saturating_sub(LIVE_ROWS);
        rows.into_iter().skip(skip).collect()
    }

    fn queued(&self, width: u16) -> Vec<Line<'static>> {
        let style = Style::new().dim();
        let mut rows: Vec<Line> = self
            .pending
            .iter()
            .take(PENDING_ROWS)
            .map(|p| {
                let one = plain(p).split_whitespace().collect::<Vec<_>>().join(" ");
                let row = format!("↳ queued: {one}");
                Line::styled(truncate(&row, width as usize), style)
            })
            .collect();
        if self.pending.len() > PENDING_ROWS {
            rows.push(Line::styled(
                format!("  +{} more", self.pending.len() - PENDING_ROWS),
                style,
            ));
        }
        rows
    }

    fn composer_rows(&self, width: u16, screen: u16) -> u16 {
        let max = (screen / 3).max(1);
        self.composer.height(width).clamp(1, max)
    }

    /// Rows the bottom pane needs on a `width`×`screen` terminal.
    pub fn height(&self, width: u16, screen: u16) -> u16 {
        let fixed = self.live(width).len() + self.queued(width).len() + 1;
        (fixed as u16 + self.composer_rows(width, screen)).min(screen)
    }

    /// Draws the bottom pane into `area`, and returns where the cursor goes.
    pub fn render(&self, area: Rect, buf: &mut Buffer, screen: u16, now: Instant) -> Position {
        let mut y = area.y;
        for row in self.live(area.width).iter().chain(&self.queued(area.width)) {
            if y + 2 > area.bottom() {
                break;
            }
            row.render(Rect::new(area.x, y, area.width, 1), buf);
            y += 1;
        }
        let rows = self
            .composer_rows(area.width, screen)
            .min(area.bottom().saturating_sub(y + 1))
            .max(1);
        let cursor = self
            .composer
            .render(Rect::new(area.x, y, area.width, rows), buf);
        let y = area.bottom().saturating_sub(1);
        let (left, right) = self.status(now);
        (&left).render(Rect::new(area.x, y, area.width, 1), buf);
        let room = (area.width as usize).saturating_sub(left.width() + 2);
        let right = truncate(&right, room);
        let x = area.right() - right.width() as u16;
        buf.set_string(x, y, right, Style::new().dim());
        cursor
    }

    /// The status bar: what runs on the left, the chat's settings and costs on the right.
    fn status(&self, now: Instant) -> (Line<'static>, String) {
        let dim = Style::new().dim();
        let left: Vec<Span> = match (self.working, self.waiting) {
            (Some(at), _) => {
                let secs = now.saturating_duration_since(at).as_secs();
                vec![
                    Span::styled(SPINNER[(secs % 4) as usize], Style::new().cyan()),
                    Span::raw(format!(" working {secs}s ")),
                    Span::styled("· Ctrl-C cancels", dim),
                ]
            }
            (None, Some(n)) => vec![Span::styled(
                format!("waiting for {} …", ai_memory::plural(n as u64, "summary")),
                Style::new().yellow(),
            )],
            (None, None) => vec![Span::styled(
                "Enter sends · Ctrl-J newline · Ctrl-D exits",
                dim,
            )],
        };
        let mut right = vec![self.model.clone(), self.effort.clone()];
        right.push(match self.prime {
            Prime::None => "unprimed".into(),
            Prime::Running => "priming …".into(),
            Prime::Done(at) => format!("primed {} ago", age(now.saturating_duration_since(at))),
            Prime::Failed => "priming failed".into(),
        });
        if self.compacting > 0 {
            right.push(format!("compacting {}", self.compacting));
        }
        if let Some(u) = self.usage {
            right.push(format!(
                "{} in · {} read · {} write · {} out",
                u.input, u.read, u.write, u.output
            ));
        }
        (Line::from(left), right.join(" · "))
    }
}

fn truncate(s: &str, width: usize) -> String {
    if s.width() <= width {
        return s.into();
    }
    if width == 0 {
        return String::new();
    }
    let mut out = String::new();
    let mut w = 0;
    for c in s.chars() {
        w += c.width().unwrap_or(0);
        if w + 1 > width {
            break;
        }
        out.push(c);
    }
    out + "…"
}

fn age(d: Duration) -> String {
    match d.as_secs() {
        s if s < 60 => format!("{s}s"),
        s => format!("{}m", s / 60),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::{Terminal, backend::TestBackend};

    fn screen(app: &App, w: u16, h: u16, now: Instant) -> Terminal<TestBackend> {
        let mut t = Terminal::new(TestBackend::new(w, h)).unwrap();
        t.draw(|f| {
            let area = f.area();
            let cursor = app.render(area, f.buffer_mut(), 20, now);
            f.set_cursor_position(cursor);
        })
        .unwrap();
        t
    }

    fn shown(t: &Terminal<TestBackend>) -> Vec<String> {
        let b = t.backend().buffer();
        b.content()
            .chunks(b.area.width as usize)
            .map(|r| r.iter().map(|c| c.symbol()).collect())
            .collect()
    }

    fn text(lines: &[Line]) -> String {
        lines
            .iter()
            .map(|l| {
                l.spans
                    .iter()
                    .map(|s| s.content.as_ref())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[test]
    fn idle_pane_with_a_draft_and_queued_messages() {
        let now = Instant::now();
        let mut app = App::new("opus", "high");
        app.show(
            Show::Primed { read: 1, write: 2 },
            now - Duration::from_secs(130),
        );
        app.show(Show::Compacting(2), now);
        app.show(
            Show::Usage(Usage {
                input: 1,
                read: 2,
                write: 3,
                output: 4,
                secs: 1.5,
            }),
            now,
        );
        app.show(Show::Waiting(3), now);
        app.show(
            Show::Pending(vec!["first\nmessage".into(), "second".into()]),
            now,
        );
        app.composer
            .paste("a draft that wraps over the edge\nand a second line");
        let height = app.height(40, 20);
        assert_eq!(height, 2 + 2 + 1);
        let t = screen(&app, 40, height, now);
        assert_eq!(
            shown(&t),
            [
                "↳ queued: first message                 ",
                "↳ queued: second                        ",
                "› a draft that wraps over the edge      ",
                "  and a second line                     ",
                "waiting for 3 summaries …  opus · high …",
            ]
        );
    }

    #[test]
    fn working_pane_shows_the_line_being_streamed() {
        let now = Instant::now();
        let mut app = App::new("opus", "high");
        app.show(Show::Model("claude-opus-5".into()), now);
        app.show(Show::Turn, now - Duration::from_secs(12));
        app.show(Show::Text("Done.\nNow the half".into()), now);
        let height = app.height(50, 20);
        let t = screen(&app, 50, height, now);
        assert_eq!(
            shown(&t),
            [
                "Now the half                                      ",
                "›                                                 ",
                "◐ working 12s · Ctrl-C cancels  claude-opus-5 · h…",
            ]
        );
        assert_eq!(text(&app.take()), "\nDone.");
    }

    #[test]
    fn history_cells_of_a_turn() {
        let now = Instant::now();
        let mut app = App::new("opus", "high");
        for s in [
            Show::View("<chat>\n0+1|note: x\n</chat>".into()),
            Show::User("what is\nup?".into()),
            Show::Turn,
            Show::Thought(42),
            Show::Call {
                name: "Bash".into(),
                input: serde_json::json!({"command": "ls -la\n  /tmp"}),
            },
            Show::Result {
                text: "a\nb\n\nc\nd\ne".into(),
                error: false,
            },
            Show::Call {
                name: "mcp__memory__zoom".into(),
                input: serde_json::json!({"id": 3, "n": 1}),
            },
            Show::Result {
                text: "boom".into(),
                error: true,
            },
            Show::Text("Here\nit is".into()),
            Show::Compactor("node 3 failed".into()),
            Show::Usage(Usage::default()),
            Show::Ready,
            Show::Unanswered("late".into()),
            Show::Cancelled,
        ] {
            app.show(s, now);
        }
        assert_eq!(
            text(&app.take()),
            "<chat>\n0+1|note: x\n</chat>\n\n› what is\n  up?\n∴ thought for ~42 tokens\n\
             • Bash ls -la /tmp\n  └ a\n    b\n    c\n    … +2 lines\n\
             • memory:zoom {\"id\":3,\"n\":1}\n  └ ✗ boom\n\nHere\ncompactor: node 3 failed\n\
             it is\n0 in · 0 read · 0 write · 0 out · 0.0 s\n\n› late  · unanswered\ncancelled"
        );
    }
}
