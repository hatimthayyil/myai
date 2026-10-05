# prov.rs

- `place`: provenance comes from the repo at the cwd when the note is taken, not the memory repo (memory is per user). `head` 12 hex, `branch` short name (`-` detached or outside a repo).
- `identity`: `<owner>/<name>` from `remote.origin.url` (last two path components, `.git` dropped), else the repo root's dir name. Not the full URL: 32 bytes, and hosts differ only in noise.
- `who`: env read through a closure so tests need not mutate process env. `AI_AGENT` wins; else `CLAUDECODE` → `claude-code`; else any of `CODEX_*` markers → `codex`. The Codex variable names are best effort (Codex documents no stable marker). `session` from `CLAUDE_CODE_SESSION_ID`, else `CODEX_THREAD_ID`, else `AI_SESSION`.
- `model`: `AI_MODEL` overrides. Else read from the calling session's transcript, since neither agent exports its model. Claude Code: `$CLAUDE_CONFIG_DIR` or `~/.claude`, `projects/*/<session>.jsonl` (the project dir is the session's start dir, so any dir matches), `message.model` of the last `"type":"assistant"` line; `<synthetic>` entries (local errors) skipped. Codex: `$CODEX_HOME` or `~/.codex`, `sessions/YYYY/MM/DD/rollout-*-<thread>.jsonl`, `payload.model` of the last `turn_context` line. Notes carry the agent's current model, so per-note lookup, not a cached value.
- `last_model`: reads the last 64 KiB, then ×16 until the whole file; finds the needle with `memmem::rfind` and parses only those lines, so lines without it cost a byte scan. Any failure yields `-`. Measured: +~0.5 ms on a real 18 MB transcript; +~28 ms on a synthetic 18 MB file whose only assistant line is the first.
- `safe_id`: the session id becomes a file name; anything but `[A-Za-z0-9-]` is refused.
- Import records carry no provenance (`-`): the importer is not the author. The commit (`import`) records who ran it.
- `repo_root` locates the starting repository's root AGENTS.md for the chat's frozen system prompt; nested cwd and worktrees use the discovered working tree.
