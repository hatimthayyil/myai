# tests/memory.rs

Port of OptMem's `test.py`, driving the library in-process (`Cli::run` with a buffer) over a synthetic life of N=2000 memories with a fake compressor (join + truncate to 280). Python checks that only made sense for the script were dropped: the shebang smoke run, the `curl | sh` bare-PATH order run (the order now names `ai memory`, which must be on PATH), and the latin-1 locale check (Rust writes bytes). Cross-process lock and the repo-local default are covered by the binary tests in `tests/cli.rs`.
