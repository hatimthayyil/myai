use ratatui::{
    style::Style,
    text::{Line, Span},
};
use serde_json::Value;

use crate::ui::plain;

const RESULT_LINES: usize = 3;
const CLIP: usize = 200;

fn clipped(s: &str) -> String {
    match s.char_indices().nth(CLIP) {
        Some((at, _)) => format!("{}…", &s[..at]),
        None => s.into(),
    }
}

fn lines(s: &str, style: Style) -> Vec<Line<'static>> {
    plain(s)
        .lines()
        .map(|l| Line::styled(l.replace('\t', "    "), style))
        .collect()
}

pub fn view(v: &str) -> Vec<Line<'static>> {
    lines(v, Style::new().dim())
}

fn message(text: &str, style: Style, note: Option<&str>) -> Vec<Line<'static>> {
    let mut out = vec![Line::default()];
    let text = plain(text);
    for (i, l) in text.lines().enumerate() {
        let prefix = if i == 0 { "› " } else { "  " };
        out.push(Line::from(vec![
            Span::styled(prefix, Style::new().cyan().bold()),
            Span::styled(l.replace('\t', "    "), style),
        ]));
    }
    if let (Some(note), Some(last)) = (note, out.last_mut()) {
        last.push_span(Span::styled(
            format!("  · {note}"),
            Style::new().dim().italic(),
        ));
    }
    out
}

pub fn user(text: &str) -> Vec<Line<'static>> {
    message(text, Style::new().bold(), None)
}

pub fn unanswered(text: &str) -> Vec<Line<'static>> {
    message(text, Style::new().dim(), Some("unanswered"))
}

/// The argument that says most about a call of tool `name`, on one line.
fn summary(name: &str, input: &Value) -> String {
    let key = match name {
        "Bash" => "command",
        "Read" | "Edit" | "Write" | "NotebookEdit" => "file_path",
        "Glob" | "Grep" => "pattern",
        "WebFetch" => "url",
        "WebSearch" => "query",
        _ => "",
    };
    let s = match input[key].as_str() {
        Some(s) => s.to_string(),
        None => input.to_string(),
    };
    clipped(&plain(&s).split_whitespace().collect::<Vec<_>>().join(" "))
}

pub fn call(name: &str, input: &Value) -> Vec<Line<'static>> {
    let name = name
        .strip_prefix("mcp__")
        .unwrap_or(name)
        .replace("__", ":");
    vec![Line::from(vec![
        Span::styled("• ", Style::new().cyan()),
        Span::styled(name.clone(), Style::new().bold()),
        Span::raw(" "),
        Span::styled(summary(&name, input), Style::new().dim()),
    ])]
}

pub fn result(text: &str, error: bool) -> Vec<Line<'static>> {
    let style = match error {
        true => Style::new().red(),
        false => Style::new().dim(),
    };
    let text = plain(text);
    let all: Vec<&str> = text.lines().filter(|l| !l.trim().is_empty()).collect();
    let mut out: Vec<Line> = all
        .iter()
        .take(RESULT_LINES)
        .enumerate()
        .map(|(i, l)| {
            let prefix = match (i, error) {
                (0, true) => "  └ ✗ ",
                (0, false) => "  └ ",
                (_, true) => "      ",
                _ => "    ",
            };
            Line::styled(
                format!("{prefix}{}", clipped(&l.replace('\t', "    "))),
                style,
            )
        })
        .collect();
    if all.is_empty() {
        out.push(Line::styled(
            if error {
                "  └ ✗ (no output)"
            } else {
                "  └ (no output)"
            },
            style,
        ));
    }
    if all.len() > RESULT_LINES {
        out.push(Line::styled(
            format!("    … +{} lines", all.len() - RESULT_LINES),
            Style::new().dim(),
        ));
    }
    out
}

pub fn thought(tokens: u64) -> Vec<Line<'static>> {
    vec![Line::styled(
        format!("∴ thought for ~{tokens} tokens"),
        Style::new().dim().italic(),
    )]
}

pub fn info(s: &str) -> Vec<Line<'static>> {
    lines(s, Style::new().dim())
}

pub fn warn(s: &str) -> Vec<Line<'static>> {
    lines(s, Style::new().yellow())
}

pub fn error(s: &str) -> Vec<Line<'static>> {
    lines(s, Style::new().red())
}
