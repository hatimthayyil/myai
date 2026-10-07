# view.rs

Spec §5–6. `Mem` = node texts per level + the view (parts tiling `[0, t)`).

- `fit(t)`: while over budget, merge the adjacent same-level pair with built parent and the largest due = `(t - start) / 2^(l+2)` (ties leftmost, f64 exact for these values). Never splits. Size = node texts only (no `id+n|`), unbuilt parts count the placeholder; kept incrementally (`set` corrects it when an unbuilt leaf in the view gets built).
- `refold`: the view is not stored; at load it is folded again from message 0 with `T = i + 1` per step over the final tree (as the reference). May differ from a live fold that ran while the compactor lagged: one cache rewrite, harmless.
- `due`: every node that is unbuilt, ready and not busy, oldest level first. No ordering against the view (OptChat's rule 3): calls carry no context, so nothing waits. `low[l]` is the first unbuilt index per level, so a scan only walks the unbuilt frontier instead of all of `T`.
- `absorb`: messages another process appended, with their already-built ancestors (only free cascades can exist there).
