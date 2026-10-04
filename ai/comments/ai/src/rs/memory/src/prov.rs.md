# prov.rs

- `place`: provenance comes from the repo at the cwd when the note is taken, not the memory repo (memory is per user). `head` 12 hex, `branch` short name (`-` detached or outside a repo).
- `identity`: `<owner>/<name>` from `remote.origin.url` (last two path components, `.git` dropped), else the repo root's dir name. Not the full URL: 32 bytes, and hosts differ only in noise.
- `who`: env read through a closure so tests need not mutate process env. `AI_AGENT` wins; else `CLAUDECODE` → `claude-code`; else any of `CODEX_*` markers → `codex`. The Codex variable names are best effort (Codex documents no stable marker). `model` only from `AI_MODEL` (Claude Code does not expose it). `session` from `CLAUDE_CODE_SESSION_ID`, else `AI_SESSION`.
- Import records carry no provenance (`-`): the importer is not the author. The commit (`import`) records who ran it.
