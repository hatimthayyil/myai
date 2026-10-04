# Cargo.toml

- No `[workspace]`: devenv's `languages.rust.import` builds crate2nix's `rootCrate`, which crate2nix only emits for a single-member workspace (`metadata.rs`: `if workspace_members.len() <= 1`). With `ai/src/rs/*` as members it only emits `workspaceMembers.*` and the import fails. Library crates are plain path dependencies instead; each declares its own versions, so shared dependencies (`clap`, `tempfile`) are repeated and must be bumped together.
- `[profile.dev.package."*"] opt-level = 3`: dependencies build optimized in dev/test. gix in debug (sha1dc, zlib-rs, odb) made the in-process test suite ~5x slower (54 s vs 10 s). Our crates stay unoptimized for debugging.
