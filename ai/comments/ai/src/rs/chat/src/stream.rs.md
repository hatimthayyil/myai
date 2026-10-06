# stream.rs

Acts carry `ui::Show` events for display (renderers format them). Stream deltas display reply text; complete assistant messages log talk and named JSON tool calls. Complete text displays when partial deltas are absent. Tool results display a short status and log capped head/tail echoes (30,000 characters plus cut notice). Initial user replay is skipped; later replays consume pending input FIFO. Thinking contents never enter memory. system/init updates model provenance and reports a disconnected memory MCP.

Tests replay two recorded real streams (`tests/fixtures/`, from shitty-optchat with the server renamed `memory`): zoom/date/Bash tool pairs then the answer, the shown text equal to the logged one; a thinking block logged nowhere, shown as its size.
