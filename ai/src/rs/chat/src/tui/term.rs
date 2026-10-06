use std::ops::Range;

use ratatui::{
    backend::{Backend, ClearType},
    buffer::Buffer,
    layout::{Position, Rect, Size},
    style::Color,
    text::Line,
    widgets::Widget,
};

use super::wrap;

/// An inline UI: a viewport of any height below the shell's output, redrawn by diffing frames,
/// with finished lines inserted above it into the terminal's own scrollback (the codex
/// `insert_history` pattern).
pub struct Term<B: Backend> {
    backend: B,
    area: Rect,
    size: Size,
    cursor: Position,
    /// What the viewport shows.
    shown: Buffer,
}

impl<B: Backend> Term<B> {
    /// Starts at the cursor's row, or below it when the row is not empty.
    pub fn new(mut backend: B) -> Result<Self, B::Error> {
        let size = backend.size()?;
        let cursor = backend.get_cursor_position()?;
        let y = (cursor.y + u16::from(cursor.x > 0)).min(size.height.saturating_sub(1));
        let area = Rect::new(0, y, size.width, 0);
        Ok(Self {
            backend,
            area,
            size,
            cursor,
            shown: Buffer::empty(area),
        })
    }

    pub fn backend_mut(&mut self) -> &mut B {
        &mut self.backend
    }

    #[cfg(test)]
    pub fn backend(&self) -> &B {
        &self.backend
    }

    #[cfg(test)]
    pub fn area(&self) -> Rect {
        self.area
    }

    pub fn width(&self) -> u16 {
        self.size.width
    }

    pub fn height(&self) -> u16 {
        self.size.height
    }

    /// Writes `lines`, wrapped, above the viewport: into free rows below it first, which pushes
    /// it down, then by scrolling the rows above it up.
    pub fn insert(&mut self, lines: &[Line<'_>]) -> Result<(), B::Error> {
        let width = self.size.width.max(1);
        let rows: Vec<Line> = lines
            .iter()
            .flat_map(|l| wrap::line(l, width as usize))
            .collect();
        if rows.is_empty() {
            return Ok(());
        }
        let n = rows.len().min(u16::MAX as usize) as u16;
        let mut buf = Buffer::empty(Rect::new(0, 0, width, n));
        for (y, row) in rows.iter().take(n as usize).enumerate() {
            row.render(Rect::new(0, y as u16, width, 1), &mut buf);
        }
        let mut area = self.area;
        let mut done = 0;
        let below = self.size.height.saturating_sub(area.bottom()).min(n);
        if below > 0 {
            self.clear_from(area.y)?;
            self.paint(&buf, 0..below, area.y)?;
            area.y += below;
            done = below;
        }
        while done < n && area.y > 0 {
            let k = (n - done).min(area.y);
            self.backend.scroll_region_up(0..area.y, k)?;
            self.paint(&buf, done..done + k, area.y - k)?;
            done += k;
        }
        self.place(area)
    }

    /// Draws `rows` of `buf` from screen row `y`, over rows known to be blank. A row ends at its
    /// last visible cell, so the terminal can reflow it on resize.
    fn paint(&mut self, buf: &Buffer, rows: Range<u16>, y: u16) -> Result<(), B::Error> {
        let w = buf.area.width;
        let mut part = Buffer::empty(Rect::new(0, y, w, rows.end - rows.start));
        for (dy, src) in rows.enumerate() {
            let end = (0..w)
                .rposition(|x| {
                    let c = &buf[(x, src)];
                    c.symbol() != " " || c.bg != Color::Reset
                })
                .map_or(0, |x| x + 1);
            for x in 0..end as u16 {
                part[(x, y + dy as u16)] = buf[(x, src)].clone();
            }
        }
        let blank = Buffer::empty(part.area);
        self.backend.draw(blank.diff(&part).into_iter())
    }

    /// Gives the viewport `height` rows, scrolling the rows above it up when it would not fit.
    pub fn fit(&mut self, height: u16) -> Result<(), B::Error> {
        let mut area = self.area;
        area.height = height.min(self.size.height);
        area.width = self.size.width;
        if area.bottom() > self.size.height {
            let over = area.bottom() - self.size.height;
            if area.y > 0 {
                self.backend.scroll_region_up(0..area.y, over.min(area.y))?;
            }
            area.y = self.size.height - area.height;
        }
        self.place(area)
    }

    /// Follows a resized screen, before anything is drawn at the old size: the viewport moves as
    /// far as the terminal moved the cursor.
    pub fn autoresize(&mut self) -> Result<(), B::Error> {
        let size = self.backend.size()?;
        if size == self.size {
            return Ok(());
        }
        let at = self.backend.get_cursor_position()?;
        let mut area = self.area;
        area.y = (area.y as i32 + at.y as i32 - self.cursor.y as i32).max(0) as u16;
        area.y = area.y.min(size.height);
        area.width = size.width;
        area.height = area.height.min(size.height);
        self.size = size;
        self.set(Rect { height: 0, ..area })?;
        self.fit(area.height)
    }

    fn place(&mut self, area: Rect) -> Result<(), B::Error> {
        match area == self.area {
            true => Ok(()),
            false => self.set(area),
        }
    }

    /// Moves the viewport to `area`, clearing the screen from its top down.
    fn set(&mut self, area: Rect) -> Result<(), B::Error> {
        self.clear_from(area.y)?;
        self.area = area;
        self.shown = Buffer::empty(area);
        Ok(())
    }

    fn clear_from(&mut self, y: u16) -> Result<(), B::Error> {
        if y >= self.size.height {
            return Ok(());
        }
        self.backend.set_cursor_position(Position::new(0, y))?;
        self.backend.clear_region(ClearType::AfterCursor)
    }

    /// Renders the viewport with `f`, which returns where the cursor goes, and writes what changed.
    pub fn draw(&mut self, f: impl FnOnce(Rect, &mut Buffer) -> Position) -> Result<(), B::Error> {
        let mut next = Buffer::empty(self.area);
        let cursor = f(self.area, &mut next);
        self.backend.draw(self.shown.diff(&next).into_iter())?;
        self.backend.set_cursor_position(cursor)?;
        self.backend.show_cursor()?;
        self.backend.flush()?;
        self.shown = next;
        self.cursor = cursor;
        Ok(())
    }

    /// Clears the viewport and leaves the cursor at its top, for what runs next.
    pub fn finish(&mut self) -> Result<(), B::Error> {
        self.clear_from(self.area.y)?;
        self.backend.show_cursor()?;
        self.backend.flush()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::{backend::TestBackend, text::Line};

    fn pane(t: &mut Term<TestBackend>, label: &str) {
        let label = label.to_string();
        t.draw(|a, buf| {
            for y in a.top()..a.bottom() {
                buf.set_string(
                    0,
                    y,
                    format!("{label}{}", y - a.y),
                    ratatui::style::Style::new(),
                );
            }
            Position::new(0, a.y)
        })
        .unwrap();
    }

    fn rows(n: usize, tag: &str) -> Vec<Line<'static>> {
        (0..n).map(|i| Line::raw(format!("{tag}{i}"))).collect()
    }

    fn scrollback(t: &Term<TestBackend>) -> Vec<String> {
        t.backend()
            .scrollback()
            .content()
            .chunks(t.width() as usize)
            .map(|r| {
                let row: String = r.iter().map(|c| c.symbol()).collect();
                row.trim_end().to_string()
            })
            .collect()
    }

    #[test]
    fn history_fills_free_rows_then_scrolls_into_scrollback() {
        let mut t = Term::new(TestBackend::new(12, 6)).unwrap();
        t.fit(2).unwrap();
        pane(&mut t, "pane");
        t.insert(&rows(3, "a")).unwrap();
        pane(&mut t, "pane");
        assert_eq!(t.area(), Rect::new(0, 3, 12, 2));
        t.insert(&rows(5, "b")).unwrap();
        t.insert(&[Line::raw("a wrapped long line")]).unwrap();
        pane(&mut t, "pane");
        assert_eq!(t.area(), Rect::new(0, 4, 12, 2));
        assert_eq!(scrollback(&t), ["a0", "a1", "a2", "b0", "b1", "b2"]);
        t.backend().assert_buffer_lines([
            "b3          ",
            "b4          ",
            "a wrapped   ",
            "long line   ",
            "pane0       ",
            "pane1       ",
        ]);
    }

    #[test]
    fn a_growing_pane_pushes_history_up_and_a_shrinking_one_clears_its_rows() {
        let mut t = Term::new(TestBackend::new(12, 6)).unwrap();
        t.fit(1).unwrap();
        t.insert(&rows(5, "a")).unwrap();
        pane(&mut t, "one");
        t.fit(3).unwrap();
        pane(&mut t, "three");
        assert_eq!(t.area(), Rect::new(0, 3, 12, 3));
        t.fit(2).unwrap();
        pane(&mut t, "two");
        assert_eq!(t.area(), Rect::new(0, 3, 12, 2));
        assert_eq!(scrollback(&t), ["a0", "a1"]);
        t.backend().assert_buffer_lines([
            "a2          ",
            "a3          ",
            "a4          ",
            "two0        ",
            "two1        ",
            "            ",
        ]);
    }
}
