# cli.rs

`ai chat` defaults to Opus at high effort; `--model` and `--effort` set the master. `ai chat mcp` skips all of that and serves the read-only tools. The system prompt (MASTER, VIEW_DOC, `~/.claude/CLAUDE.md`, the repo's `AGENTS.md`) is read once.

The chat takes `compact.lock` before anything else; if a nap or another chat holds it, it says so once and waits (polling each second) instead of failing: a note-spawned nap ends soon.

Input: when stdin and stdout are both terminals, the TUI (`tui`) owns the terminal and turns keys into events (Ctrl-C cancel, Ctrl-D on an empty composer exit, Enter a message, a bracketed paste stays one message). Otherwise a plain line reader thread, one message per line. SIGINT is a cancel (raw mode means Ctrl-C arrives as a key in the TUI); SIGTERM/SIGHUP an exit. At the end of stdin: a terminal (Ctrl-D) exits at once; a pipe ends after the queued work, so `printf … | ai chat` runs its turns. Children run in their own process groups, so the terminal's SIGINT reaches only the chat. A broken stdout exits quietly; the owners' destructors end every model call.
