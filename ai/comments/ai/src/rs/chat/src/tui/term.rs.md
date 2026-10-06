# tui/term.rs

Inline terminal, after codex-rs/tui (`custom_terminal.rs`, `insert_history.rs`, Apache-2.0): the pattern, not its code.

- Not `ratatui::Terminal`: its inline viewport has a fixed height, and its `Viewport::Fixed` resize resets the viewport to row 0 and clears the whole screen whenever the width shrinks. `Term` keeps the last frame (`shown`) and writes `Buffer::diff` itself through the ratatui `Backend` (works with `TestBackend` for tests).
- History insert: lines are pre-wrapped (`wrap::line`) and drawn into the free rows below the viewport first (moving it down), then by `scroll_region_up(0..top)` (DECSTBM + SU, ratatui's `scrolling-regions` feature), which pushes the top rows into native scrollback. Cells are drawn over blank rows by diffing against an empty buffer, which handles wide characters. Each row stops at its last visible cell so the terminal can reflow it on a width change (a dimmed line otherwise writes styled spaces to the full width).
- Viewport height follows the pane (`fit`): growing scrolls the rows above up; shrinking leaves blank rows below until history fills them.
- Resize: checked before every frame (`autoresize`, ioctl size), never after drawing at the old size, which would let the terminal clamp the cursor and scroll. The viewport moves by as much as the terminal moved the cursor (codex's heuristic), then is fitted again, scrolling history up rather than overwriting it.
- `finish` clears the viewport and leaves the cursor at its top, so the shell prompt follows the history.
