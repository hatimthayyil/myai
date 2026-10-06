use std::sync::LazyLock;

use pulldown_cmark::{CodeBlockKind, Event, HeadingLevel, Options, Parser, Tag, TagEnd};
use ratatui::{
    style::{Color, Modifier, Style},
    text::{Line, Span},
};
use syntect::{
    easy::HighlightLines,
    highlighting::{FontStyle, Theme, ThemeSet},
    parsing::SyntaxSet,
    util::LinesWithEndings,
};

static SYNTAXES: LazyLock<SyntaxSet> = LazyLock::new(SyntaxSet::load_defaults_newlines);
static THEME: LazyLock<Theme> = LazyLock::new(|| {
    ThemeSet::load_defaults()
        .themes
        .remove("base16-ocean.dark")
        .unwrap_or_default()
});
const RULE: usize = 32;

/// The highlighted lines of the code block last seen, kept so a block that grows line by line
/// is highlighted once per line.
struct Code {
    lang: String,
    done: String,
    lines: Vec<Line<'static>>,
    h: HighlightLines<'static>,
}

impl Code {
    fn new(lang: &str) -> Self {
        let syntax = SYNTAXES
            .find_syntax_by_token(lang)
            .unwrap_or_else(|| SYNTAXES.find_syntax_plain_text());
        Self {
            lang: lang.into(),
            done: String::new(),
            lines: Vec::new(),
            h: HighlightLines::new(syntax, &THEME),
        }
    }

    fn line(&mut self, l: &str) -> Line<'static> {
        let spans = match self.h.highlight_line(l, &SYNTAXES) {
            Ok(parts) => parts
                .into_iter()
                .map(|(s, t)| Span::styled(t.trim_end_matches('\n').to_string(), style(s)))
                .collect(),
            Err(_) => vec![Span::raw(l.trim_end_matches('\n').to_string())],
        };
        Line::from(spans)
    }
}

fn style(s: syntect::highlighting::Style) -> Style {
    let c = s.foreground;
    let mut out = Style::new().fg(Color::Rgb(c.r, c.g, c.b));
    if s.font_style.contains(FontStyle::BOLD) {
        out = out.add_modifier(Modifier::BOLD);
    }
    if s.font_style.contains(FontStyle::ITALIC) {
        out = out.add_modifier(Modifier::ITALIC);
    }
    if s.font_style.contains(FontStyle::UNDERLINE) {
        out = out.add_modifier(Modifier::UNDERLINED);
    }
    out
}

/// Renders markdown for the terminal: styled inline text, bullets, quotes and highlighted code.
/// Line breaks in the source stay line breaks, so lines already shown never change.
#[derive(Default)]
pub struct Markdown {
    code: Option<Code>,
}

/// One rendering, with the start of its last top-level block: byte offset and first line.
pub struct Rendered {
    pub lines: Vec<Line<'static>>,
    pub last: Option<(usize, usize)>,
}

impl Markdown {
    pub fn render(&mut self, src: &str) -> Rendered {
        let mut w = Writer::default();
        let mut depth = 0usize;
        let opts = Options::ENABLE_STRIKETHROUGH | Options::ENABLE_TASKLISTS;
        for (ev, range) in Parser::new_ext(src, opts).into_offset_iter() {
            match ev {
                Event::Start(tag) => {
                    if depth == 0 {
                        w.gap();
                        w.last = Some((range.start, w.lines.len()));
                    }
                    depth += 1;
                    self.start(&mut w, tag);
                }
                Event::End(tag) => {
                    depth = depth.saturating_sub(1);
                    self.end(&mut w, tag);
                }
                Event::Text(t) if w.code.is_some() => w.code.as_mut().unwrap().1.push_str(&t),
                Event::Text(t) => {
                    if let Some((_, text)) = w.link.as_mut() {
                        text.push_str(&t);
                    }
                    w.text(&t, w.style());
                }
                Event::Code(t) => w.text(&t, w.style().fg(Color::Cyan)),
                Event::InlineMath(t) | Event::DisplayMath(t) => w.text(&t, w.style()),
                Event::Html(t) | Event::InlineHtml(t) => w.text(&t, w.style()),
                Event::FootnoteReference(t) => w.text(&format!("[^{t}]"), w.style()),
                Event::SoftBreak | Event::HardBreak => w.newline(),
                Event::Rule => {
                    if depth == 0 {
                        w.gap();
                        w.last = Some((range.start, w.lines.len()));
                    }
                    w.text(&"─".repeat(RULE), Style::new().dim());
                    w.newline();
                }
                Event::TaskListMarker(done) => {
                    w.text(if done { "[x] " } else { "[ ] " }, w.style());
                }
            }
        }
        w.flush();
        Rendered {
            lines: w.lines,
            last: w.last,
        }
    }

    fn start(&mut self, w: &mut Writer, tag: Tag) {
        match tag {
            Tag::Heading { level, .. } => w.styles.push(match level {
                HeadingLevel::H1 => Style::new().bold().underlined().fg(Color::Cyan),
                HeadingLevel::H2 => Style::new().bold().fg(Color::Cyan),
                _ => Style::new().bold(),
            }),
            Tag::BlockQuote(_) => {
                w.flush();
                w.prefix.push("│ ".into());
                w.styles.push(Style::new().fg(Color::Green));
            }
            Tag::CodeBlock(kind) => {
                w.flush();
                let lang = match kind {
                    CodeBlockKind::Fenced(l) => l.split_whitespace().next().unwrap_or("").into(),
                    CodeBlockKind::Indented => String::new(),
                };
                w.code = Some((lang, String::new()));
            }
            Tag::List(first) => {
                w.flush();
                w.lists.push(first);
            }
            Tag::Item => {
                w.flush();
                let marker = match w.lists.last_mut() {
                    Some(Some(n)) => {
                        *n += 1;
                        format!("{}. ", *n - 1)
                    }
                    _ => "• ".into(),
                };
                w.prefix.push(" ".repeat(marker.chars().count()));
                w.marker = Some(marker);
            }
            Tag::Emphasis => w.styles.push(w.style().italic()),
            Tag::Strong => w.styles.push(w.style().bold()),
            Tag::Strikethrough => w.styles.push(w.style().crossed_out()),
            Tag::Link { dest_url, .. } | Tag::Image { dest_url, .. } => {
                w.styles.push(w.style().underlined());
                w.link = Some((dest_url.to_string(), String::new()));
            }
            _ => {}
        }
    }

    fn end(&mut self, w: &mut Writer, tag: TagEnd) {
        match tag {
            TagEnd::Paragraph => w.flush(),
            TagEnd::Heading(_) => {
                w.styles.pop();
                w.flush();
            }
            TagEnd::BlockQuote(_) => {
                w.flush();
                w.prefix.pop();
                w.styles.pop();
            }
            TagEnd::CodeBlock => {
                if let Some((lang, code)) = w.code.take() {
                    for line in self.highlight(&lang, &code) {
                        w.push_line(line);
                    }
                }
            }
            TagEnd::List(_) => {
                w.flush();
                w.lists.pop();
            }
            TagEnd::Item => {
                w.flush();
                w.prefix.pop();
                w.marker = None;
            }
            TagEnd::Emphasis | TagEnd::Strong | TagEnd::Strikethrough => {
                w.styles.pop();
            }
            TagEnd::Link | TagEnd::Image => {
                w.styles.pop();
                if let Some((url, text)) = w.link.take()
                    && url != text
                    && !url.is_empty()
                {
                    w.text(&format!(" ({url})"), Style::new().dim());
                }
            }
            _ => {}
        }
    }

    /// The lines of a code block, continuing the last highlighting when this block extends it.
    fn highlight(&mut self, lang: &str, code: &str) -> Vec<Line<'static>> {
        let c = match self.code.take() {
            Some(c) if c.lang == lang && code.starts_with(&c.done) => c,
            _ => Code::new(lang),
        };
        let c = self.code.insert(c);
        let rest = code[c.done.len()..].to_string();
        let mut tail = Vec::new();
        for l in LinesWithEndings::from(&rest) {
            let line = c.line(l);
            match l.ends_with('\n') {
                true => {
                    c.done.push_str(l);
                    c.lines.push(line);
                }
                false => tail.push(line),
            }
        }
        c.lines.iter().cloned().chain(tail).collect()
    }
}

#[derive(Default)]
struct Writer {
    lines: Vec<Line<'static>>,
    cur: Vec<Span<'static>>,
    styles: Vec<Style>,
    prefix: Vec<String>,
    marker: Option<String>,
    lists: Vec<Option<u64>>,
    code: Option<(String, String)>,
    link: Option<(String, String)>,
    last: Option<(usize, usize)>,
}

impl Writer {
    fn style(&self) -> Style {
        self.styles.last().copied().unwrap_or_default()
    }

    /// The indent of a new line: quote bars and item indents, or the item's marker on its first line.
    fn lead(&mut self) -> Vec<Span<'static>> {
        let n = self.prefix.len();
        let marker = self.marker.take();
        self.prefix
            .iter()
            .enumerate()
            .map(|(i, p)| match (&marker, i + 1 == n) {
                (Some(m), true) => Span::styled(m.clone(), Style::new().fg(Color::Cyan)),
                _ if p.starts_with('│') => Span::styled(p.clone(), Style::new().fg(Color::Green)),
                _ => Span::raw(p.clone()),
            })
            .collect()
    }

    fn text(&mut self, s: &str, style: Style) {
        for (i, part) in s.split('\n').enumerate() {
            if i > 0 {
                self.newline();
            }
            if part.is_empty() {
                continue;
            }
            if self.cur.is_empty() {
                self.cur = self.lead();
            }
            self.cur.push(Span::styled(part.to_string(), style));
        }
    }

    fn newline(&mut self) {
        let mut spans = std::mem::take(&mut self.cur);
        if spans.is_empty() {
            spans = self.lead();
        }
        self.lines.push(Line::from(spans));
    }

    fn push_line(&mut self, line: Line<'static>) {
        let mut spans = self.lead();
        spans.extend(line.spans);
        self.lines.push(Line::from(spans));
    }

    fn flush(&mut self) {
        if !self.cur.is_empty() {
            self.newline();
        }
    }

    /// A blank line between top-level blocks.
    fn gap(&mut self) {
        self.flush();
        if !self.lines.is_empty() {
            self.lines.push(Line::default());
        }
    }
}
