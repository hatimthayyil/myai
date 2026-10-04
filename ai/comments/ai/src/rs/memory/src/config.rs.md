# config.rs

- Knobs in the memory repo's git config (`ai.memory.partChars`, `partLines`): output paging only. Per machine. `WAKE_LINES` and `ENTRY_CHARS` are gone: the view budget is the spec's constant `VIEW` (the chat must fold the same view), and the note limit derives from `NODE`.
- `Config` holds only overrides; unknown `ai.memory.*` keys are ignored.
