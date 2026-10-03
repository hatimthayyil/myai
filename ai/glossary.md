# Glossary

- `memory`: the store `ai memory` manages; one identity, append-only. Default `<repo root>/.ai/memory`, override `AI_MEMORY_DIR`. Also: one recorded line in it.
- `record`: fixed-width slot on disk. Log record = 320 bytes, tree record = 288; text padded with spaces, ends in `\n`. Position is identity.
- `block`: aligned power-of-two range of memories `[lo, hi)`, printed inclusive as `lo-(hi-1)`. Blocks of size ≥ 2 hold a one-line summary in `TREE/<size>`.
- `cover`: the set of blocks `wake` prints: tiles `[0, T)`, at most `WAKE_LINES` blocks, finer toward the present.
- `wake`: print the cover, paged into parts. Refuses while a needed summary is missing.
- `part`: one page of `wake` output, bounded by `PART_LINES`/`PART_CHARS`.
- `nap`: build the next pending summary; blocks are built in order, smallest first.
- `zoom`: open one block into its two halves.
- `forget`: drop a summary and everything built on it; `nap` rebuilds them. Never touches the log.
- `recall`: case-insensitive regex search over every raw memory, newest matches kept within `PART_CHARS`.
- `knob`: a per-memory size in `config`: `WAKE_LINES`, `ENTRY_CHARS`, `PART_CHARS`, `PART_LINES`.
- `provenance`: origin of a data item: who/what produced it, when, from which session, model and commit.
- `attribution`: authorship of work (code, files) as AI-generated, AI-assisted or human.
