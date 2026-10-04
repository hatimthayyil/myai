# merge.rs

R3 of `ai/research/20261003T091838---memory-git-ref.md`.

- `merge`: `theirs` ancestor of ours (or equal) = nothing to do. Ours absent, ours ancestor of theirs, or equal trees = move the ref to theirs, no commit (racing syncs converge without a merge chain). Otherwise a commit with parents `[ours, theirs]`. CAS; a lost race redoes from the new head, no refetch. No merge base: unrelated histories (two `init`s) merge the same way.
- `union`, log: sorted union by key. Equal keys are one memory; differing bytes under one key mean corruption: keep the smaller, count it (`clashes`), warn. Deterministic, so every clone computes the same log.
- `union`, tree: per level, a pool `fp -> record` from both sides at any offset; on an fp clash the max `(ts, origin, bytes)` wins (LWW, bytes only break exact ties). Each level is rebuilt as a dense prefix of blocks whose fp is in the pool, capped at half the level below so a block never outlives its halves. Missing blocks become pending; `nap` rebuilds them.
- `write`: rewrite a level only from its first changed record; unchanged segments keep their blob ids.
- `first_change`: the first local position whose key moved; reported as "positions from #p renumbered". Appends alone renumber nothing.
- Logs are read whole (512 B per memory). Fine at tens of thousands; streaming is the fix if it ever matters.
