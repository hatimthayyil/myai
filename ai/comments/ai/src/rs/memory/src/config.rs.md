# config.rs

- Knobs in the memory repo's git config (`ai.memory.entryChars`, `partChars`, `partLines`). Per machine. `ENTRY_CHARS`: the note limit, default and maximum `NOTE_MAX` (506: `note: ` + text fits one node). `WAKE_LINES` is gone: the view budget is the spec's constant `VIEW` (the chat must fold the same view).
- `ai.memory.origin` lives in the same section but is no knob: store.rs owns it.
- `Config` holds only overrides; unknown `ai.memory.*` keys are ignored.
