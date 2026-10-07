# Compactor eval: one model vs another

Each model rebuilds the full summary tree from the same notes; compare the trees.

1. Per model `m`: copy `~/.ai/memory` to `<dir>/m`, then commit on it a tree with `tree/1..` removed (only level-0 leaves kept). Plumbing: `git archive` the ref into a work dir, `rm -rf tree/[1-9]*`, `add -A` with a temp `GIT_INDEX_FILE`, `write-tree`, `commit-tree -p <old>`, `update-ref`.
2. Build: `AI_MEMORY_NAP=0 AI_MEMORY_MODEL=<model id> AI_MEMORY_DIR=<dir>/m ai memory nap` (time it).
3. Extract: `git --git-dir <dir>/m archive refs/ai/memory tree | tar -x -C <dir> --one-top-level=t-m`, then `uv run --no-project python keep.py haiku sonnet` from `<dir>` (copy `keep.py` there): hard-fact keep rate per level.
4. Read `wake` at `WAKE_LINES=16` per copy (`ai memory config WAKE_LINES=16`), and the top node `tree/6/0000/00`.
5. Check faithfulness: grep the summaries for the example line's content (`scale.txt`: export.py, --verbose, write_header, ship tonight) and for `[cut]`/`LIMIT` markers; count lines not ending in punctuation (truncated on retry).

2026-10-08 result (83 notes): Sonnet 5.5 207 s, no invented content, 37/79 lines truncated; Haiku 5.5 454 s, more hard facts kept per level (59/36/18% vs 53/28/13% at 4/8/16 notes) but copied scale.txt content into 3 lines, 23/79 truncated. Sonnet stays default.
