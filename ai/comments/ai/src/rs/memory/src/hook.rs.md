# hook.rs

- `ai memory hook`: the UserPromptSubmit hook harnesses call, so the reminder text and its threshold ship with the product, not in each user's harness config.
- Input: the harness's hook JSON on stdin. Claude Code and Codex send an object with `prompt` (plus `session_id`, `cwd`, `hook_event_name`, ...); pi's extension sends `{"prompt": ...}`. Only `prompt` is read; serde ignores the rest.
- Long prompt (`LONG_PROMPT` = 150 chars, counted as `char`s, not bytes): prints `{"hookSpecificOutput":{"hookEventName":"UserPromptSubmit","additionalContext":REMINDER}}`, the shape Claude Code and Codex both accept. Short prompt: prints nothing. serde_json sorts the keys; order does not matter to the harness.
- `REMINDER` is `prompts/reminder.txt`, trailing whitespace trimmed at compile time (`trim_ascii_end` is const). No config knob.
- Never blocks: malformed input (not JSON, no string `prompt`, unreadable stdin) prints nothing and exits 0. The error is kept in the usage log (`ok: false`, `error`). A missing store still fails like every other command (exit 1, which Claude Code and Codex treat as non-blocking): the hook is only installed beside a memory, and logging before `Store::open` could drop `usage.jsonl` into a non-memory dir (cli.rs.md, Usage log).
- `REMINDED`: the usage log's argument for a call that printed the reminder, so stats can relate reminders to recall. Shared by cli.rs (writes) and stats.rs (reads).
