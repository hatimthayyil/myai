# nap.rs

- `RAW_MAX`: blocks up to this many memories compress from the raw log; larger ones from their two halves' summaries.
- `pending`: each level file holds a dense prefix, so its length says exactly how far that level got: one stat per level, never a scan.
- `pending_count`: a level can hold more blocks than T needs (T is a snapshot; memories keep arriving while an agent reads), so each level clamps at zero.
- `prompt`: `pending` lists a block only after its halves settled, so a missing half is a blank record (a corrupt write): point at `forget`.
