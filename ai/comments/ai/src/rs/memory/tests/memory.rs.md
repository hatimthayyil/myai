# tests/memory.rs

In-process library and CLI tests (`Cli::run_with` with a fake `Runtime`).

- `Fake`: a backend whose reply is a function of the step text and attempt; it asserts what every call must look like (system = COMPACT, cached `<chat>` pieces with no ids or placeholders, unmarked step, retry message ends `| ← LIMIT`) and records order and peak concurrency. Summaries are 300 bytes so two never merge free.
- View invariants (spec §5, ported from shitty-optchat's selfcheck): 1200 random messages on a 6000-byte budget, nodes built synchronously in rule-3 order: tiles `[0,T)`, size bookkeeping exact, never over budget while a mergeable pair exists, never splits, refold equals the live fold, stays near budget.
- Rule 3, free nodes and cascades, zoom semantics, placeholder rendering, compactor order/concurrency/settle, view under budget while logging, notes from another process picked up, size retries (shortest of 5), failure reported once then retried, the compactor lock, and a CLI life (note limits, import, wake pinned across parts, nap, zoom, show, grep paging).
- Lock race: `before_release` appends a note and runs a second nap while the first still holds the lock; the second must report the lock taken and the first must build the note's nodes before exiting.
- Message timezone rendering and raw zoom paging preserve UTF-8/newlines and reject invalid part numbers.
