# devenv.nix

- devenv is the dev shell only. The package is defined once, in `flake.nix` (see `flake.nix.md`).
- `enterTest` (`devenv test`): root `cargo test` does not test the library crates, since cargo refuses to test a non-member path dependency that has dev-dependencies (`cargo test -p ai-memory` errors). Each `ai/src/rs/*` crate is tested via `--manifest-path`, sharing the root `target/`. Those runs write a per-crate `Cargo.lock`, which `.gitignore` excludes; the root `Cargo.lock` is the one that counts.
