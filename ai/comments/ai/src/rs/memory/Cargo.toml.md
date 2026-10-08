# ai/src/rs/memory/Cargo.toml

- `[profile.dev.package."*"]` duplicates the root's: profiles only apply from the manifest cargo is invoked on, and this crate's tests run via `--manifest-path` (see `devenv.nix.md`). Use `CARGO_TARGET_DIR=<root>/target` so no `target/` appears here.
- `gix`: `default-features = false`, `sha1` and `anyhow`. No network: sync is gone.
- `serde_json`: stream-json with Claude Code. `tempfile`: the backend's system-prompt files.
- `getrandom`: the store's origin.
- `libc`: Linux child process groups and parent-death signals.
- `tracing`, `tracing-subscriber` (`fmt`, `env-filter`, `std`; no `ansi`, no proc-macro attributes): the nap log, see `src/log.rs.md`.
- `backon` (`default-features = false`, `std`): retry backoff for the compactor, used as an iterator of delays; no Tokio or timers pulled in. Its async `Retryable` (feature `tokio-sleep`) is the path if the compactor moves to Tokio. `backoff` is unmaintained (RUSTSEC-2025-0012, which points to backon). See ai/research/*rust-retry-crates.md.
