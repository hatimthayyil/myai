# store.rs

Spec: `ai/research/20261003T091838---memory-git-ref.md` (R1, R2) and its addendum `20261004T152738---memory-user-level.md`.

- The memory is a bare git repo (default `~/.ai/memory`); all data lives as objects under `REF` (`refs/ai/memory`). No working files, so nothing to grep, ignore or repair.
- Layout: `log/<hi>/<lo>` and `tree/<size>/<hi>/<lo>`, segment `n` = records `[256n, 256n+256)`, path `{n>>8:04x}/{n&255:02x}` (`seg_path`). Two-level fanout keeps every tree object ≤256 entries; a note rewrites one tail blob, three small trees and a commit. Hex names of fixed width sort numerically, so the last entry is the tail.
- `ME`: every printed command must run as printed; the tool names itself `ai memory`.
- `pretty`: a path as the user would type it; keeps symlinks (no canonicalize), folds `$HOME` to `~`.
- `AtPath`: report filesystem failures as `<path>: <strerror>.`, never a backtrace. Strips Rust's ` (os error N)` suffix.
- `Store::open`: only `init` creates a memory. A missing dir, a non-repo or a non-bare repo all read as "No memory": creating one on demand would let a typo in `AI_MEMORY_DIR` open an empty identity. Committer falls back to `ai memory <ai@localhost>` when git config has none, so commits never fail on a fresh machine.
- `Store::create`: idempotent. Refuses a non-empty dir that is not a memory (e.g. an old file store): no migration, and never init over foreign files.
- `Snapshot`: one commit. Every command reads one snapshot, so all parts of a command see the same state. Segments are cached per snapshot (wake reads a few segments many times). Sync builds on `Store::at(commit)`, `commit` and `cas`.
- `count`: length of a level = last segment number × 256 + last segment size / 512, from the object header (no inflate). Only the tail segment can be partial.
- `scan`: a search reads a whole level by nature but must not hold it; it fetches one segment at a time, bypassing the cache, and stops at `end` (grep's `--before`). `log_scan` and `tree_scan` decode on top of it.
- `tree_get` / `tree_scan` (via `summary`): absent record = not built yet; non-UTF-8 = corrupt, point at `forget`; blank text = `None` (callers report a blank summary; `tree_scan` skips it). Returns the whole `Summary` so `show` can print its provenance.
- `commit` / `cas`: the two halves of a mutation, public so a merge can write a two-parent commit, or fast-forward without one. `cas` is false only when another writer moved the ref.
- `is_ancestor`: walk from `a` with `b` hidden; nothing left means `a` is reachable from `b`.
- `setting`: resolved git config (global included); empty reads as unset.
- `dump`: a whole level as one buffer, uncached; only a merge reads everything.
- `replace`: truncate then append, so a merge rewrites a level from its first changed record only.
- `mutate`: replaces the old `flock`. Build blobs/trees/commit from a snapshot, then update the ref with CAS (`MustExistAndMatch`, or `MustNotExist` for the first commit). On failure, if the ref moved the attempt lost the race: redo from the new snapshot. If it did not move, the error is real (gix already waits ~100 ms for a held ref lock, `core.filesRefLockTimeout`). Empty `Changes` = no commit.
- `Changes`: whole segment blobs by path; an empty blob removes the path (segments are never empty), and gix prunes empty trees.
- `edit_config`: git config writes go through `config.lock` like git does; the file is re-read under the lock.
- `origin`: random 30 bits as 6 Crockford chars, in `ai.memory.origin`. Created under the config lock and re-checked there, so two first writers agree on one id.
- `log_append`: the only way the log grows. Keys `(ts, origin)` are assigned inside the CAS from the tail, so concurrent notes never collide; the caller's `ts` is only the wanted time (now, or an imported date's midnight).
- `tree_put`: blocks are built in order; `Moved` (no commit) if the level moved on, `Changed` if the members' fingerprint no longer starts with the prompt's `@fp` (a sync landed). Both checks run inside the CAS. `fp` covers the member keys, read from the same snapshot.
- `tree_drop`: truncate each level back to the dropped block. Later blocks at those levels go too and are rebuilt; the log is never touched.
