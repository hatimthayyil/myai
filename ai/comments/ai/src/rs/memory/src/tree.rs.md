# tree.rs

- `Coord`: node `(l, i)` covers `[i·2^l, (i+1)·2^l)`, printed `id+n` (spec §3). `at` validates `n` a power of two, `id % n == 0`, `id + n ≤ T`.
- `free`: a source that fits `NODE` (512) bytes is its own node (spec §3 free nodes). `joined` is the free merge (space, see record.rs).
