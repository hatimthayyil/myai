# flake.nix

- The single package definition for `ai` (devenv is only the dev shell). Install and update:
  - `nix profile add /hatimthayyil/code/myai` (profile element is named `myai`)
  - `nix profile upgrade myai`
  - Build locally: `nix build` → `./result/bin/ai`
- A local path with `.git` is fetched as `git+file`, so only git-tracked files reach the store (`target/` never does). New files must be `git add`ed before nix sees them. `src` is further narrowed with `lib.fileset` to the Rust sources, so doc edits do not trigger rebuilds.
- `rustPlatform.buildRustPackage` with `cargoLock.lockFile`: no generated Nix, no extra inputs. Replaced devenv's crate2nix-based `languages.rust.import`. nixos-unstable's rustc supports edition 2024.
- `forAllSystems` is `lib.genAttrs` over the four common systems; flake-utils not needed.
- Checks: all three suites run. The root suite runs via `cargoCheckHook`; `postCheck` runs the memory and chat suites via `--manifest-path` (path dependencies, not workspace members), reusing the build's target dir and target triple so dependencies are not recompiled. Their `Cargo.lock` is copied from the root, since per-crate locks are untracked.
- `git` is the only extra check input. The fake `claude` scripts take their shebang from `bash` on `PATH` (stdenv provides it), since the sandbox has no `/usr/bin/env`.
