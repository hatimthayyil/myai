# config.rs

- `Knob`: sizes a memory may override in its own `config` file. `PART_*` are transport limits, not memory limits: every harness truncates long output differently (Claude Code cuts the middle at 30,000 chars, pi the head at 50 KB, Codex budgets 10,000 tokens), so output is handed over in parts that fit all of them. `WAKE_LINES` 96 ≈ 8k tokens of dense text, in 2 parts.
- `Knob::validate`: a bad knob stops every command, so the message says where it is written (`<file> line N:`); `config` cannot fix a file it also refuses to read. `ENTRY_CHARS` is capped so a memory fits both record widths.
- `Config`: holds only overrides; an unset knob keeps the tool's default, so updating the tool still changes behaviour.
- `Config::write`: every knob on its own line, commented out unless overridden. Same layout as the Python tool.
