# config.rs

- `Knob`: sizes a memory may override, stored in the memory repo's git config as `ai.memory.<key>` (`wakeLines`, `entryChars`, `partChars`, `partLines`). Per machine, never synced: they are reading budgets tied to the harness on that machine. The user-facing names stay `WAKE_LINES` etc. `PART_*` are transport limits: every harness truncates long output differently (Claude Code cuts the middle at 30,000 chars, pi the head at 50 KB, Codex budgets 10,000 tokens), so output is handed over in parts that fit all of them. `WAKE_LINES` 96 ≈ 8k tokens of dense text, in 2 parts.
- `Knob::validate`: a bad knob stops every command, so the message names the file and key (`<dir>/config: ai.memory.wakeLines`). `ENTRY_CHARS` is capped at `TEXT_MAX`.
- `Config`: holds only overrides; an unset knob keeps the tool's default, so updating the tool still changes behaviour. Unknown `ai.memory.*` keys are ignored, as git does (`origin` and `remote` live there too).
- `Config::load` reads the resolved config, so a knob in `~/.gitconfig` also applies.
- `Config::save`: sets overrides, removes the rest.
