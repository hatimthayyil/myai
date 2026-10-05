# ai/src/rs/memory/Cargo.toml

- `[profile.dev.package."*"]` duplicates the root's: profiles only apply from the manifest cargo is invoked on, and this crate's tests run via `--manifest-path` (see `devenv.nix.md`). Use `CARGO_TARGET_DIR=<root>/target` so no `target/` appears here.
- `gix`: `default-features = false`, `sha1` and `anyhow`. No network: sync is gone.
- `serde_json`: stream-json with Claude Code. `tempfile`: the backend's system-prompt files.
- `libc`: Linux child process groups and parent-death signals.
