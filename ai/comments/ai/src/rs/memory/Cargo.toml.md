# ai/src/rs/memory/Cargo.toml

- `[profile.dev.package."*"]` duplicates the root's: profiles only apply from the manifest cargo is invoked on, and this crate's tests run via `--manifest-path` (see `devenv.nix.md`). Without it gix builds unoptimized and the suite takes ~60 s.
- `gix`: `default-features = false`, `sha1` (object hash) and `anyhow` (its `Exn` errors convert into `anyhow::Error`). Fetch/push are not used; sync shells out to `git` for them, so ssh config, credential helpers and `insteadOf` just work (jj made the same split). Ancestry checks use `rev_walk` with a hidden tip instead of `merge_base`, which needs the `revision` feature and pulls in `gix-index`.
