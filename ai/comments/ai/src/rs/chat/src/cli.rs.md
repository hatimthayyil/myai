# cli.rs

`ai chat` defaults to Opus at high effort; `--model` and `--effort` set the master. `ai chat mcp` skips all of that and serves the read-only tools. The system prompt (MASTER, VIEW_DOC, `~/.claude/CLAUDE.md`, the repo's `AGENTS.md`) is read once.

The chat takes `compact.lock` before anything else; if a nap or another chat holds it, it says so once and waits (polling each second) instead of failing: a note-spawned nap ends soon.

Input: plain line reader on its own thread, one message per line, no line editor (the terminal's own scrollback and line editing). SIGINT is a cancel; SIGTERM/SIGHUP an exit. At the end of stdin: a terminal (Ctrl-D) exits at once; a pipe ends after the queued work, so `printf … | ai chat` runs its turns. Children run in their own process groups, so the terminal's SIGINT reaches only the chat. A broken stdout exits quietly; the owners' destructors end every model call.
