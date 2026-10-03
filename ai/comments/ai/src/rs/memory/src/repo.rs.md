# repo.rs

- `root`: one discovery function, shared by `Location::find` (default store) and `init` (gitignore). A `.git` file counts, so worktrees and submodules resolve to their own root.
- `ignore_store`: the repo-local store is per-machine state; `init` keeps it out of git. Idempotent: any trimmed exact form of the path (`/.ai/memory/`, `/.ai/memory`, `.ai/memory`, `.ai/memory/`) counts as present. Appends after a newline if the file lacks a trailing one. Only runs when the store is the repo default, never for `AI_MEMORY_DIR`.
