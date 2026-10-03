# cover.rs

- `tile`: Python `_cover`. Tile `[0,T)` with aligned power-of-two blocks; a block stays whole iff its size ≤ `alpha` × its age (`T - lo`). Bigger alpha = coarser = fewer lines.
- `cover`: detail decays with age, so recent memories stay verbatim and ancient ones collapse; if everything fits, nothing is compressed. 60-step bisection on alpha finds the finest tiling within budget. Block sizes jump in powers of two, so alpha alone can undershoot the budget: the top-up splits the newest splittable block, spending leftover lines on the present.
- tests: `complete(T)` is the oracle (every buildable block, straight from the definition). `work_never_spikes`: one new memory creates at most 16 naps.
