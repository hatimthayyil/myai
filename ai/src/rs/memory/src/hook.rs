use std::{
    io::{self, Write},
    path::Path,
};

use anyhow::{Context, Result};
use serde::Deserialize;
use serde_json::json;

use crate::{record::Who, usage};

pub const REMINDER: &str = include_str!("../prompts/reminder.txt").trim_ascii_end();
pub const LONG_PROMPT: usize = 150;
pub const REMIND_EVERY: usize = 5;
/// The usage log's argument for a hook call that printed the reminder.
pub const REMINDED: &str = "reminded";
/// The usage log's argument for a reminder due to a run of unreminded prompts.
pub const PERIODIC: &str = "periodic";
const LONG: &str = "long";

#[derive(Deserialize)]
struct Input {
    prompt: String,
    #[serde(default)]
    session_id: String,
}

/// Prints the reminder as the harness's extra context when `input`'s prompt is long, or when
/// the session's last `REMIND_EVERY - 1` prompts in the usage log in `dir` went without one;
/// the rule that fired. Sets `who`'s session from the input when it names one.
pub fn hook(
    out: &mut dyn Write,
    input: io::Result<String>,
    dir: &Path,
    who: &mut Who,
) -> Result<Option<&'static str>> {
    let input: Input = serde_json::from_str(&input.context("Cannot read the hook input.")?)
        .context("Not a hook input with a prompt.")?;
    if !input.session_id.is_empty() {
        who.session = input.session_id;
    }
    let rule = if input.prompt.chars().count() >= LONG_PROMPT {
        LONG
    } else if unreminded(dir, &who.session) {
        PERIODIC
    } else {
        return Ok(None);
    };
    let reply = json!({
        "hookSpecificOutput": {
            "hookEventName": "UserPromptSubmit",
            "additionalContext": REMINDER,
        }
    });
    writeln!(out, "{reply}")?;
    Ok(Some(rule))
}

fn unreminded(dir: &Path, session: &str) -> bool {
    let n = REMIND_EVERY - 1;
    !session.is_empty()
        && usage::last(dir, n, |u| u.cmd == "hook" && u.ok && u.session == session).is_ok_and(
            |hooks| hooks.len() == n && hooks.iter().all(|u| !u.args.iter().any(|a| a == REMINDED)),
        )
}
