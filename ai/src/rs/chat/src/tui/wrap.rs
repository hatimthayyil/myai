use std::ops::Range;

use ratatui::text::{Line, Span};
use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;

/// The byte ranges of the rows `s` (one line) takes at `width` columns: greedy word wrap after
/// spaces, words wider than a row split anywhere.
pub fn rows(s: &str, width: usize) -> Vec<Range<usize>> {
    let width = width.max(1);
    let mut rows = Vec::new();
    let (mut start, mut col) = (0, 0);
    let mut brk: Option<(usize, usize)> = None;
    for (i, g) in s.grapheme_indices(true) {
        let w = g.width();
        if col + w > width && col > 0 {
            match brk.filter(|&(b, _)| b > start) {
                Some((b, at)) => {
                    rows.push(start..b);
                    start = b;
                    col -= at;
                }
                None => {
                    rows.push(start..i);
                    start = i;
                    col = 0;
                }
            }
            brk = None;
        }
        col += w;
        if g == " " {
            brk = Some((i + 1, col));
        }
    }
    rows.push(start..s.len());
    rows
}

/// `line` cut into rows of at most `width` columns, keeping every span's style.
pub fn line(line: &Line<'_>, width: usize) -> Vec<Line<'static>> {
    let text: String = line.spans.iter().map(|s| s.content.as_ref()).collect();
    rows(&text, width)
        .into_iter()
        .map(|r| {
            let mut spans = Vec::new();
            let mut at = 0;
            for s in &line.spans {
                let (a, b) = (at, at + s.content.len());
                at = b;
                let (lo, hi) = (a.max(r.start), b.min(r.end));
                if lo < hi {
                    spans.push(Span::styled(s.content[lo - a..hi - a].to_string(), s.style));
                }
            }
            Line::from(spans).style(line.style)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::style::Stylize;

    fn cut(s: &str, w: usize) -> Vec<&str> {
        rows(s, w).into_iter().map(|r| &s[r]).collect()
    }

    #[test]
    fn wraps_after_spaces_and_splits_long_words() {
        assert_eq!(cut("", 4), [""]);
        assert_eq!(cut("ab cd ef", 5), ["ab ", "cd ef"]);
        assert_eq!(cut("abcdefgh", 3), ["abc", "def", "gh"]);
        assert_eq!(cut("a bcdefgh", 3), ["a ", "bcd", "efg", "h"]);
        assert_eq!(cut("日本語", 4), ["日本", "語"]);
    }

    #[test]
    fn styled_lines_keep_their_spans() {
        let l = Line::from(vec![Span::raw("ab "), Span::raw("cd").bold()]);
        let rows = line(&l, 4);
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[1].spans[0].content, "cd");
        assert_eq!(rows[1].spans[0].style, l.spans[1].style);
    }
}
