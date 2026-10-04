# devenv.nix

- `outputs.ai`: packaged with devenv's documented `languages.rust.import` (crate2nix; needs the `crate2nix` input). Build with `devenv build outputs.ai`. Requires the root crate to be the only workspace member; see `Cargo.toml.md`.
- `enterTest` (`devenv test`): root `cargo test` does not test the library crates, since cargo refuses to test a non-member path dependency that has dev-dependencies (`cargo test -p ai-memory` errors). Each `ai/src/rs/*` crate is tested via `--manifest-path`, sharing the root `target/`. Those runs write a per-crate `Cargo.lock`, which `.gitignore` excludes; the root `Cargo.lock` is the one that counts.
