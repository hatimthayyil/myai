use std::{cell::Cell, ops::Range};

use ratatui::{
    buffer::Buffer,
    crossterm::event::{KeyCode, KeyEvent, KeyModifiers},
    layout::{Position, Rect},
    style::Style,
};
use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;

use super::wrap;

const PROMPT: &str = "› ";

/// What a key asks of the chat beyond editing.
#[derive(Debug, PartialEq, Eq)]
pub enum Action {
    Submit(String),
    Cancel,
    Exit,
}

/// The message being written: multiline, soft-wrapped, with the session's sent messages as history.
#[derive(Default)]
pub struct Composer {
    text: String,
    at: usize,
    history: Vec<String>,
    browsing: Option<usize>,
    draft: String,
    width: Cell<usize>,
}

impl Composer {
    #[cfg(test)]
    pub fn text(&self) -> &str {
        &self.text
    }

    pub fn paste(&mut self, s: &str) {
        self.insert(&s.replace("\r\n", "\n").replace('\r', "\n"));
    }

    pub fn key(&mut self, k: KeyEvent) -> Option<Action> {
        let ctrl = k.modifiers.contains(KeyModifiers::CONTROL);
        let alt = k.modifiers.contains(KeyModifiers::ALT);
        match k.code {
            KeyCode::Char('c') if ctrl => return Some(Action::Cancel),
            KeyCode::Char('d') if ctrl && self.text.is_empty() => return Some(Action::Exit),
            KeyCode::Enter
                if k.modifiers
                    .intersects(KeyModifiers::SHIFT | KeyModifiers::ALT) =>
            {
                self.insert("\n")
            }
            KeyCode::Char('j') if ctrl => self.insert("\n"),
            KeyCode::Enter => return self.submit(),
            KeyCode::Char('a') if ctrl => self.at = self.line_start(),
            KeyCode::Home => self.at = self.line_start(),
            KeyCode::Char('e') if ctrl => self.at = self.line_end(),
            KeyCode::End => self.at = self.line_end(),
            KeyCode::Char('b') if ctrl => self.at = self.prev(),
            KeyCode::Char('f') if ctrl => self.at = self.next(),
            KeyCode::Char('b') if alt => self.at = self.word_left(),
            KeyCode::Char('f') if alt => self.at = self.word_right(),
            KeyCode::Left if ctrl || alt => self.at = self.word_left(),
            KeyCode::Right if ctrl || alt => self.at = self.word_right(),
            KeyCode::Left => self.at = self.prev(),
            KeyCode::Right => self.at = self.next(),
            KeyCode::Char('d') if ctrl => self.cut(self.at..self.next()),
            KeyCode::Delete => self.cut(self.at..self.next()),
            KeyCode::Backspace if ctrl || alt => self.cut(self.word_left()..self.at),
            KeyCode::Char('w') if ctrl => self.cut(self.word_left()..self.at),
            KeyCode::Char('h') if ctrl => self.cut(self.prev()..self.at),
            KeyCode::Backspace => self.cut(self.prev()..self.at),
            KeyCode::Char('u') if ctrl => self.cut(self.line_start()..self.at),
            KeyCode::Char('k') if ctrl => self.cut(self.at..self.line_end()),
            KeyCode::Up => self.up(),
            KeyCode::Char('p') if ctrl => self.up(),
            KeyCode::Down => self.down(),
            KeyCode::Char('n') if ctrl => self.down(),
            KeyCode::Tab => self.insert("\t"),
            KeyCode::Char(c) if !ctrl && !alt => self.insert(c.encode_utf8(&mut [0; 4])),
            _ => {}
        }
        None
    }

    fn submit(&mut self) -> Option<Action> {
        let text = std::mem::take(&mut self.text);
        self.at = 0;
        self.browsing = None;
        if text.trim().is_empty() {
            return None;
        }
        if self.history.last() != Some(&text) {
            self.history.push(text.clone());
        }
        Some(Action::Submit(text))
    }

    fn insert(&mut self, s: &str) {
        self.text.insert_str(self.at, s);
        self.at += s.len();
    }

    fn cut(&mut self, r: Range<usize>) {
        self.at = r.start;
        self.text.replace_range(r, "");
    }

    fn prev(&self) -> usize {
        self.text[..self.at]
            .grapheme_indices(true)
            .next_back()
            .map_or(0, |(i, _)| i)
    }

    fn next(&self) -> usize {
        self.text[self.at..]
            .graphemes(true)
            .next()
            .map_or(self.at, |g| self.at + g.len())
    }

    fn line_start(&self) -> usize {
        self.text[..self.at].rfind('\n').map_or(0, |i| i + 1)
    }

    fn line_end(&self) -> usize {
        self.text[self.at..]
            .find('\n')
            .map_or(self.text.len(), |i| self.at + i)
    }

    fn word_left(&self) -> usize {
        let head = self.text[..self.at].trim_end();
        head.rfind(char::is_whitespace).map_or(0, |i| i + 1)
    }

    fn word_right(&self) -> usize {
        let tail = &self.text[self.at..];
        let skip = tail.len() - tail.trim_start().len();
        tail[skip..]
            .find(char::is_whitespace)
            .map_or(self.text.len(), |i| self.at + skip + i)
    }

    fn up(&mut self) {
        if !self.move_row(-1) && !self.history.is_empty() {
            let i = match self.browsing {
                Some(0) => return,
                Some(i) => i - 1,
                None => {
                    self.draft = self.text.clone();
                    self.history.len() - 1
                }
            };
            self.browsing = Some(i);
            self.text = self.history[i].clone();
            self.at = self.text.len();
        }
    }

    fn down(&mut self) {
        if !self.move_row(1)
            && let Some(i) = self.browsing
        {
            if i + 1 < self.history.len() {
                self.browsing = Some(i + 1);
                self.text = self.history[i + 1].clone();
            } else {
                self.browsing = None;
                self.text = std::mem::take(&mut self.draft);
            }
            self.at = self.text.len();
        }
    }

    /// Moves to the visual row `by` rows away, at the last width rendered.
    fn move_row(&mut self, by: isize) -> bool {
        let rows = self.rows(self.width.get());
        let (row, col) = self.cursor(&rows, self.width.get());
        let Some(r) = row.checked_add_signed(by).and_then(|r| rows.get(r)) else {
            return false;
        };
        let mut at = r.start;
        let mut w = 0;
        for (i, g) in self.text[r.clone()].grapheme_indices(true) {
            w += g.width();
            if w > col {
                break;
            }
            at = r.start + i + g.len();
        }
        self.at = at;
        true
    }

    /// The byte ranges of the visual rows at `width` text columns.
    fn rows(&self, width: usize) -> Vec<Range<usize>> {
        let mut rows = Vec::new();
        let mut start = 0;
        for line in self.text.split('\n') {
            rows.extend(
                wrap::rows(line, width)
                    .into_iter()
                    .map(|r| start + r.start..start + r.end),
            );
            start += line.len() + 1;
        }
        rows
    }

    /// The visual row and column of the cursor; past the last row when that row is full.
    fn cursor(&self, rows: &[Range<usize>], width: usize) -> (usize, usize) {
        let row = rows.iter().rposition(|r| r.start <= self.at).unwrap_or(0);
        let col = self.text[rows[row].start..self.at].width();
        match col >= width {
            true => (row + 1, 0),
            false => (row, col),
        }
    }

    /// Rows needed to show it all at `width` columns.
    pub fn height(&self, width: u16) -> u16 {
        let w = text_width(width);
        let rows = self.rows(w);
        let (row, _) = self.cursor(&rows, w);
        rows.len().max(row + 1) as u16
    }

    /// Draws the rows that keep the cursor in view, and returns the cursor's position.
    pub fn render(&self, area: Rect, buf: &mut Buffer) -> Position {
        let w = text_width(area.width);
        self.width.set(w);
        let rows = self.rows(w);
        let (row, col) = self.cursor(&rows, w);
        let top = (row + 1).saturating_sub(area.height as usize);
        for (y, i) in (top..rows.len().max(row + 1)).enumerate() {
            if y as u16 >= area.height {
                break;
            }
            let prefix = if i == 0 { PROMPT } else { "  " };
            buf.set_string(
                area.x,
                area.y + y as u16,
                prefix,
                Style::new().cyan().bold(),
            );
            if let Some(r) = rows.get(i) {
                let shown = self.text[r.clone()].replace('\t', " ");
                buf.set_stringn(area.x + 2, area.y + y as u16, shown, w, Style::new());
            }
        }
        Position::new(area.x + 2 + col as u16, area.y + (row - top) as u16)
    }
}

fn text_width(width: u16) -> usize {
    (width as usize).saturating_sub(PROMPT.width()).max(1)
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::crossterm::event::KeyEventKind;

    fn press(c: &mut Composer, code: KeyCode, mods: KeyModifiers) -> Option<Action> {
        c.key(KeyEvent::new_with_kind(code, mods, KeyEventKind::Press))
    }

    fn typed(s: &str) -> Composer {
        let mut c = Composer::default();
        for ch in s.chars() {
            press(&mut c, KeyCode::Char(ch), KeyModifiers::NONE);
        }
        c
    }

    #[test]
    fn newline_keys_and_a_paste_make_one_message() {
        let mut c = typed("a");
        press(&mut c, KeyCode::Enter, KeyModifiers::SHIFT);
        press(&mut c, KeyCode::Enter, KeyModifiers::ALT);
        press(&mut c, KeyCode::Char('j'), KeyModifiers::CONTROL);
        c.paste("b\r\nc");
        assert_eq!(c.text(), "a\n\n\nb\nc");
        assert_eq!(
            press(&mut c, KeyCode::Enter, KeyModifiers::NONE),
            Some(Action::Submit("a\n\n\nb\nc".into()))
        );
        assert_eq!(c.text(), "");
        assert_eq!(press(&mut c, KeyCode::Enter, KeyModifiers::NONE), None);
    }

    #[test]
    fn ctrl_c_cancels_and_ctrl_d_exits_only_when_empty() {
        let mut c = typed("ab");
        assert_eq!(
            press(&mut c, KeyCode::Char('c'), KeyModifiers::CONTROL),
            Some(Action::Cancel)
        );
        press(&mut c, KeyCode::Home, KeyModifiers::NONE);
        assert_eq!(
            press(&mut c, KeyCode::Char('d'), KeyModifiers::CONTROL),
            None
        );
        assert_eq!(c.text(), "b");
        press(&mut c, KeyCode::Char('k'), KeyModifiers::CONTROL);
        assert_eq!(
            press(&mut c, KeyCode::Char('d'), KeyModifiers::CONTROL),
            Some(Action::Exit)
        );
    }

    #[test]
    fn editing_moves_by_graphemes_and_words() {
        let mut c = typed("héllo big world");
        press(&mut c, KeyCode::Char('w'), KeyModifiers::CONTROL);
        assert_eq!(c.text(), "héllo big ");
        press(&mut c, KeyCode::Left, KeyModifiers::CONTROL);
        press(&mut c, KeyCode::Left, KeyModifiers::NONE);
        press(&mut c, KeyCode::Backspace, KeyModifiers::NONE);
        assert_eq!(c.text(), "héll big ");
        press(&mut c, KeyCode::Char('a'), KeyModifiers::CONTROL);
        press(&mut c, KeyCode::Right, KeyModifiers::NONE);
        press(&mut c, KeyCode::Delete, KeyModifiers::NONE);
        assert_eq!(c.text(), "hll big ");
    }

    #[test]
    fn up_and_down_walk_rows_then_history() {
        let mut c = typed("one");
        press(&mut c, KeyCode::Enter, KeyModifiers::NONE);
        c.paste("two\nthree");
        press(&mut c, KeyCode::Enter, KeyModifiers::NONE);
        c.paste("draft");
        c.render(
            Rect::new(0, 0, 40, 5),
            &mut Buffer::empty(Rect::new(0, 0, 40, 5)),
        );
        press(&mut c, KeyCode::Up, KeyModifiers::NONE);
        assert_eq!(c.text(), "two\nthree");
        press(&mut c, KeyCode::Up, KeyModifiers::NONE);
        press(&mut c, KeyCode::Char('!'), KeyModifiers::NONE);
        assert_eq!(c.text(), "two!\nthree");
        press(&mut c, KeyCode::Up, KeyModifiers::NONE);
        assert_eq!(c.text(), "one");
        press(&mut c, KeyCode::Down, KeyModifiers::NONE);
        press(&mut c, KeyCode::Down, KeyModifiers::NONE);
        press(&mut c, KeyCode::Down, KeyModifiers::NONE);
        assert_eq!(c.text(), "draft");
    }

    #[test]
    fn height_counts_wrapped_rows_and_a_cursor_past_a_full_row() {
        let c = typed("abcd efgh");
        assert_eq!(c.height(2 + 10), 1);
        assert_eq!(c.height(2 + 9), 2);
        assert_eq!(c.height(2 + 5), 2);
        let c = typed("abcde");
        assert_eq!(c.height(2 + 5), 2);
        assert_eq!("\t".width(), 1);
    }
}
