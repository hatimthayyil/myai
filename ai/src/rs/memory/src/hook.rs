use std::io::{self, Write};

use anyhow::{Context, Result};
use serde::Deserialize;
use serde_json::json;

pub const REMINDER: &str = include_str!("../prompts/reminder.txt").trim_ascii_end();
pub const LONG_PROMPT: usize = 150;
/// The usage log's argument for a hook call that printed the reminder.
pub const REMINDED: &str = "reminded";

#[derive(Deserialize)]
struct Input {
    prompt: String,
}

/// Prints the reminder as the harness's extra context when `input`'s prompt is long; whether it did.
pub fn hook(out: &mut dyn Write, input: io::Result<String>) -> Result<bool> {
    let input: Input = serde_json::from_str(&input.context("Cannot read the hook input.")?)
        .context("Not a hook input with a prompt.")?;
    if input.prompt.chars().count() < LONG_PROMPT {
        return Ok(false);
    }
    let reply = json!({
        "hookSpecificOutput": {
            "hookEventName": "UserPromptSubmit",
            "additionalContext": REMINDER,
        }
    });
    writeln!(out, "{reply}")?;
    Ok(true)
}
