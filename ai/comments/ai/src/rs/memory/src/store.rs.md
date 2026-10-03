# store.rs

- `LOG_REC`/`TREE_REC`: records are fixed width, so a memory or block is found by seeking: memory i at `i*LOG_REC` of `LOG.txt`, block `[k*s,(k+1)*s)` at `k*TREE_REC` of `TREE/<s>`. Padding costs ~2x on disk and buys O(1) everywhere.
- `ME`: every printed command must run as printed; the tool names itself `ai memory`.
- `pretty`: a path as the user would type it; keeps symlinks (no canonicalize), folds `$HOME` to `~`.
- `AtPath`: the filesystem is the one thing the tool does not control; report failures as `<path>: <strerror>.` like every other message, never a backtrace. Strips Rust's ` (os error N)` suffix to get the bare strerror.
- `Entry::decode` / `records`: records are sliced as bytes and decoded one by one; slicing decoded text would shift every boundary after the first multi-byte character.
- `count`: only NotFound means zero; any other failure is real and must surface (an unreadable level must not read as pending work).
- `repair`: drop a partial trailing record left by a crash. It was never acknowledged. Without this the next append lands at a wrong offset and every later record is misaligned. Callers hold the lock.
- `Store::open`: only `init` creates the directory. Creating it is creating the identity; if any command created it, a typo in `AI_MEMORY_DIR` would silently open an empty store and the agent would wake with no past.
- `Store::create`: idempotent: creates only what is missing (TREE, LOG.txt opened for append, config when absent); never truncates or rewrites.
- `lock`: `.lock` opened in append mode (not truncate) and locked with `File::lock` (flock); released on drop.
- `log_scan`: a search reads the whole log by nature but must not hold it: at a million memories that is 300 MB. Streams 4096 records per read.
- `tree_get`: missing level file = not built yet. Invalid UTF-8 is a corrupt record: point at `forget`.
- `log_append`: the only way `LOG.txt` changes. Ids assigned inside the lock so concurrent notes never share an id. fsync before returning.
- `tree_put`: blocks are built in order, so this only ever appends one record to one level file; refuses if the level moved on (settled or forgotten meanwhile).
- `tree_drop`: truncate each level back to the dropped block. Later blocks at those levels go too and are rebuilt; the log is never touched, so nothing is lost.
