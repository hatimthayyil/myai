# sync.rs

R3 of `ai/research/20261003T091838---memory-git-ref.md`, remote per the addendum `20261004T152738---memory-user-level.md`.

- `remote`: `ai.memory.remote`, else `origin` when `remote.origin.url` is set; none = local only, no network.
- `Git::run`: transport through the `git` CLI. `GIT_TERMINAL_PROMPT=0`; ssh gets `ConnectTimeout=3` and `BatchMode=yes` unless the user set `GIT_SSH`, `GIT_SSH_COMMAND` or `core.sshCommand`. `LC_ALL=C` so the one message parsed (`couldn't find remote ref`: the remote has no memory yet) is stable. A deadline kills git; its stderr reader is left behind, since an orphaned ssh may hold the pipe. The reason shown is git's first `fatal:`/`error:` line.
- `sync`: fetch into `refs/ai/remotes/<remote>/memory` (`--no-write-fetch-head`: nothing else written to the memory dir), merge the tracking ref, push `refs/ai/memory` without force. Fetch failure = offline: merge whatever was fetched before, warn, no push. A rejected push (the remote moved) refetches and remerges, up to 3 times. Nothing to send = no push. Then `git gc --auto --quiet`: gix never collects.
- `Report`: `taken` compares the snapshot before and after, so it covers fast-forwards and merges alike; `redo` is every pending summary afterwards.
