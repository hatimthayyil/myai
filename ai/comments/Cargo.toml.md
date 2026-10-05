# Cargo.toml

- No `[workspace]`: originally forced by crate2nix (devenv's `languages.rust.import` only emitted `rootCrate` for a single-member workspace). The flake's `buildRustPackage` has no such limit, so a workspace is now possible; until then, library crates are plain path dependencies, each declaring its own versions, so shared dependencies (`clap`, `tempfile`) are repeated and must be bumped together.
- `[profile.dev.package."*"] opt-level = 3`: dependencies build optimized in dev/test. gix in debug (sha1dc, zlib-rs, odb) made the in-process test suite ~5x slower (54 s vs 10 s). Our crates stay unoptimized for debugging.
