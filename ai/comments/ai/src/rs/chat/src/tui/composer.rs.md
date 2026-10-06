# tui/composer.rs

Own composer instead of a crate. Checked (2026-10-06): `ratatui-textarea` 0.9.3 (ratatui org fork, depends on ratatui-core 0.1, so ratatui 0.30) soft-wraps (`WrapMode::WordOrGlyph`) but exposes no wrapped row count, which a growing inline composer needs to size its viewport; `tui-textarea` 0.7 is on ratatui 0.29; `tui-textarea-2` 0.13 is similar to the first. Codex's textarea is ~4.7k lines. This one is a `String` plus a byte cursor, laid out with `wrap::rows`, so height, rendering and cursor position share one layout.

Keys: Enter sends; Shift/Alt-Enter and Ctrl-J insert a newline (Ctrl-J also works without the keyboard protocol: raw mode reads LF as Ctrl-J); Ctrl-C cancels; Ctrl-D exits on an empty composer, else deletes forward; Emacs-style moves and kills (Ctrl-A/E/B/F/K/U/W/H, Alt-B/F, Ctrl/Alt-arrows, Alt-Backspace); Up/Down move between visual rows, then walk this session's sent messages (the draft is kept). A bracketed paste is inserted as text (CR/CRLF to LF), so it is sent as one message with Enter. Tabs stay tabs in the message and show as one column.

The cursor past a full row shows at the start of the next row (an extra row is counted).
