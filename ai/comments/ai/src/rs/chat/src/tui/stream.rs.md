# tui/stream.rs

Commits complete lines only (codex `markdown_stream` idea): at each new newline it renders the source up to it and sends the rendered lines not yet sent; the line being written shows raw in the pane. After each pass it rebases to the start of the last top-level block, since earlier blocks can no longer change, so a pass costs one block. The test streams a reply in 1/2/3/7-character pieces and requires the same lines as one whole render.
