# Backlog

## Memory

- `ai chat` stages 1–2 implemented and smoke-tested against real Claude Code (sonnet; one prime + one turn). Installed (flake, profile entry `ai`); live memory rebuilt with full provenance. Pending: measure real subscription usage (compaction especially) in the first week. Spec: `ai/research/20261004T194110---ai-chat.md`.
- `ai chat` TUI shipped in 0.4.0, user-tested (chatting, turns saved); compaction not yet exercised live. Gaps: no Ctrl-Z suspend; input history and drafts are per session; code colours assume a dark truecolor terminal; tables raw; the streaming line shows raw markdown; resize checked in tmux only.
- `ai chat` frozen (2026-10-08) until Victor Taelin open-sources his OptChat; then redesign. One session burned ~35% of usage (tool calls logged, whole-view compactor context, Opus re-priming).
- `ai memory` compactor prompt defects fixed (retry asks for a complete rewrite; `scale.txt` is lorem ipsum): truncation 37→2 (Sonnet), 23→6 (Haiku), no leaks. Haiku 5.5 still distorts merges (`ai/evals/compactor`, 2026-10-08b); Sonnet stays default.
- `ai memory`: test `parallel_processes_lose_nothing` is flaky (timestamp ordering); failed once in 4 runs.
- pi extension: `ai memory` as pi's context engine, with pi as the agent. `context` hook replaces messages with the view plus recent messages; `session_before_compact` cancels pi compaction (the `ai memory` compactor owns it); `message_end`/`tool_execution_end` log to the store; `registerTool` for zoom/date; pi's cache warming replaces priming. Precedent: pi-blackhole. For models off the Claude subscription (API key, ChatGPT, local): pi bills Claude Pro/Max as per-token extra usage (`pi/packages/coding-agent/docs/providers.md`). Needs store and compactor usable from outside `ai chat`, and Session emitting events (TUI work).
- Multi-machine sync for the chat memory (rebase-style merge: pushed messages never move), deferred.
- Repo-level memory, alongside the user memory. Deferred. Decided (2026-10-04):
  - Purpose: portable with the repo, shared with collaborators, focus at wake. Inside a repo, repo memory has priority; user memory stays reachable.
  - Routing: every note goes to user memory. Notes that are repo-local and not personal also go to the repo memory, as the same record (same key).
  - Wake: both covers in one `WAKE_LINES` budget, repo first.
  - Sharing: one ref per person (`refs/ai/memory/<person>`) on the project remote; teammates' refs readable.
  - Open: who decides the repo copy (agent `note --repo` vs classifier, below); budget split; how teammates' refs appear (wake, grep/zoom, or not yet); creation and push policy; `<person>` identity; scope prefixes on ids (`u#`, `r#`).
- Classifier ("Jev type model") to pick the notes that also go to repo memory: repo-local and not personal. Deferred until repo memory exists.
- `ai memory` summaries carry `note:`/`user:` tags copied from the leaves (code labels each leaf `note: text`); Sonnet ignores an instruction to drop them. Wasted bytes per item; decide whether leaves should be unlabelled or tags kept only for `user:` words. See ai/evals/compactor/README.md 2026-10-08c.
