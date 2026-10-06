# tui/app.rs

Pure state: `show` turns `Show`s into history lines (`take`) and pane state; `key` turns keys into session events; `height`/`render` draw the bottom pane. No I/O, so it is tested by rendering into a `TestBackend` and comparing the text.

- Pane, top to bottom: the reply line being streamed (last 3 rows), pending messages (`↳ queued:`, 3 rows and `+N more`), the composer (up to a third of the screen), the status bar.
- Status bar: left is what runs (spinner and elapsed seconds, "waiting for N summaries", or key hints); right is model (updated from the call's init), effort, priming state and age, compactor calls running, last turn's usage. The right side is truncated first.
- Turn-content events end the streamed reply first (its last line goes to history); background events (compactor reports, priming) are inserted without breaking a reply line.
- Priming success shows only in the status bar; `Plain` prints it.
