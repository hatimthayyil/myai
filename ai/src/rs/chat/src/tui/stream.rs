use ratatui::text::Line;

use super::markdown::Markdown;
use crate::ui::plain;

/// Streamed reply text as markdown: the rendering of its complete lines goes to history as it
/// grows, the line being written stays live. Each pass renders from the last top-level block on,
/// since the blocks before it can no longer change.
#[derive(Default)]
pub struct Stream {
    md: Markdown,
    src: String,
    /// Where the blocks still rendered start, and where the line being written starts.
    base: usize,
    complete: usize,
    /// Lines of the rendering from `base` already in history.
    done: usize,
}

impl Stream {
    pub fn push(&mut self, t: &str) -> Vec<Line<'static>> {
        let mut out = Vec::new();
        if self.src.is_empty() {
            out.push(Line::default());
        }
        self.src.push_str(&plain(t));
        if let Some(end) = self.src.rfind('\n').filter(|&e| e >= self.complete) {
            self.complete = end + 1;
            let r = self.md.render(&self.src[self.base..self.complete]);
            out.extend(r.lines.iter().skip(self.done).cloned());
            self.done = r.lines.len();
            if let Some((at, line)) = r.last.filter(|&(at, _)| at > 0) {
                self.base += at;
                self.done -= line;
            }
        }
        out
    }

    /// The line being written.
    pub fn tail(&self) -> &str {
        &self.src[self.complete..]
    }

    /// Ends the reply: what is left of its rendering goes to history too.
    pub fn finish(&mut self) -> Vec<Line<'static>> {
        let rest = match self.complete < self.src.len() {
            true => {
                let r = self.md.render(&self.src[self.base..]);
                r.lines.into_iter().skip(self.done).collect()
            }
            false => Vec::new(),
        };
        *self = Self {
            md: std::mem::take(&mut self.md),
            ..Self::default()
        };
        rest
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(lines: &[Line]) -> Vec<String> {
        lines
            .iter()
            .map(|l| l.spans.iter().map(|s| s.content.as_ref()).collect())
            .collect()
    }

    const REPLY: &str = "# Title\n\nSome **bold** and `code`,\na [link](http://x).\n\n\
        - one\n- two\n  1. nested\n\n> quoted\n> twice\n\n```rust\nfn main() {\n    let x = 1;\n}\n```\n\n---\nDone.";

    #[test]
    fn streamed_in_any_pieces_it_renders_as_a_whole() {
        let whole = {
            let mut s = Stream::default();
            let mut out = s.push(REPLY);
            out.extend(s.finish());
            text(&out)
        };
        let rule = "─".repeat(32);
        assert_eq!(
            whole,
            [
                "",
                "Title",
                "",
                "Some bold and code,",
                "a link (http://x).",
                "",
                "• one",
                "• two",
                "  1. nested",
                "",
                "│ quoted",
                "│ twice",
                "",
                "fn main() {",
                "    let x = 1;",
                "}",
                "",
                rule.as_str(),
                "",
                "Done.",
            ]
        );
        for size in [1, 2, 3, 7] {
            let mut s = Stream::default();
            let mut out = Vec::new();
            let chars: Vec<char> = REPLY.chars().collect();
            for piece in chars.chunks(size) {
                out.extend(s.push(&piece.iter().collect::<String>()));
            }
            out.extend(s.finish());
            assert_eq!(text(&out), whole, "pieces of {size}");
        }
    }

    #[test]
    fn only_complete_lines_are_committed() {
        let mut s = Stream::default();
        assert_eq!(text(&s.push("Hel")), [""]);
        assert_eq!(s.tail(), "Hel");
        assert_eq!(text(&s.push("lo\nwor")), ["Hello"]);
        assert_eq!(s.tail(), "wor");
        assert_eq!(text(&s.finish()), ["wor"]);
        assert_eq!(text(&s.push("x\n")), ["", "x"]);
    }
}
