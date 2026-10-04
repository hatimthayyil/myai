# Cargo.toml

- `[profile.dev.package."*"] opt-level = 3`: dependencies build optimized in dev/test. gix in debug (sha1dc, zlib-rs, odb) made the in-process test suite ~5x slower (54 s vs 10 s). Our crates stay unoptimized for debugging.
- `gix`: `default-features = false`, `sha1` (object hash) and `anyhow` (its `Exn` errors convert into `anyhow::Error`). Fetch/push are not used; sync shells out to `git` for them, so ssh config, credential helpers and `insteadOf` just work (jj made the same split). Ancestry checks use `rev_walk` with a hidden tip instead of `merge_base`, which needs the `revision` feature and pulls in `gix-index`.
