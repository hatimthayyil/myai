# tests/memory.rs

Port of OptMem's `test.py`, driving the library in-process (`Cli::run` with a buffer) over a synthetic life of N=2000 memories with a fake compressor (join + truncate to 280). Python checks that only made sense for the script were dropped: the shebang smoke run, the `curl | sh` bare-PATH order run (the order now names `ai memory`, which must be on PATH), and the latin-1 locale check (Rust writes bytes). Torn-write and permission checks went with the file store: git objects are written whole.

Store state is inspected through the library (`Store`, `Snapshot`); corruption is injected with `Store::mutate` (`rewrite`). Cross-process CAS, provenance, commit messages and the home default are covered by the binary tests in `tests/cli.rs`.

Navigation: `zoom` descends 1024 → 128 → 16 → raw in 3 calls, checking every printed id is zoomable and the frontier tiles its block; refusal names a fitting depth; a raw block is never refused. `grep` paging follows the printed `--before` footers until every hit was seen exactly once, for the log and with `-t` (summaries interleave, so ties in position are exercised).
