# store.rs

Spec: `ai/research/20261004T194110---ai-chat.md` (storage layout). Bare repo per user (`~/.ai/memory`, `AI_MEMORY_DIR`), all data under `REF`.

- Layout: message `i` is its own blob at `log/{i>>8:04x}/{i&255:02x}` (header line `ts kind repo head branch agent model session`, then the text verbatim). Nodes of level `l` live in line segments `tree/<l>/{s>>8:04x}/{s&255:02x}`, segment `s` = nodes `[256s, 256s+256)`, one line per node (`ts model text`, text flattened), empty line = not built. Messages are variable size (echoes up to 30k, pastes larger), so each gets its own blob: a tail segment of big messages would be rewritten on every append. Nodes are small and read all at once at load (refold), so they are segmented; levels are sparse (free nodes and parallel merges land out of order), hence empty lines rather than a dense prefix.
- `log_len`: the last entry's number + 1, from two tree reads. Ids are positions and permanent: single machine, append-only.
- `mutate`: one commit per mutation, CAS on the ref; a lost race redoes from the new head. Multi-writer safe (notes from many sessions, one compactor).
- `Edit`: one mutation's new messages and nodes over a snapshot. Every node write cascades: the parent is built in the same commit when both children exist and `a + " " + b` fits `NODE` (free node). `push` builds the free leaf (`kind: text` ≤ `NODE`). So after any commit every free node whose sources exist is built, and no compactor needs a free-node pass.
- `put_node`: first write wins (a built node never changes); refuses a node beyond the log.
- Free nodes record model `-`.
- `commit_id`: `wake`'s pin is a full commit hex; no prefix lookup (would need gix's revision feature).
- `pretty`, `AtPath`: paths as the user types them; `<path>: <strerror>.` errors.
- `Store::open` refuses a missing/non-repo dir (only `init` creates); committer falls back to `ai memory <ai@localhost>`.
