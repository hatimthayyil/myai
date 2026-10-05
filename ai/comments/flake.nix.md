# flake.nix

- The single package definition for `ai` (devenv is only the dev shell). Install and update:
  - `nix profile add /hatimthayyil/code/myai` (profile element is named `myai`)
  - `nix profile upgrade myai`
  - Build locally: `nix build` → `./result/bin/ai`
- A local path with `.git` is fetched as `git+file`, so only git-tracked files reach the store (`target/` never does). New files must be `git add`ed before nix sees them. `src` is further narrowed with `lib.fileset` to the Rust sources, so doc edits do not trigger rebuilds.
- `rustPlatform.buildRustPackage` with `cargoLock.lockFile`: no generated Nix, no extra inputs. Replaced devenv's crate2nix-based `languages.rust.import`. nixos-unstable's rustc supports edition 2024.
- `forAllSystems` is `lib.genAttrs` over the four common systems; flake-utils not needed.
- Checks: the root suite runs via `cargoCheckHook`; `postCheck` runs the memory suite via `--manifest-path` (path dependency, not a workspace member), reusing the build's target dir and target triple so dependencies are not recompiled. Its `Cargo.lock` is copied from the root, since per-crate locks are untracked.
- Not run in the sandbox: the nix sandbox has no `/usr/bin/env`, and the fake `claude` scripts use `#!/usr/bin/env bash`.
  - Root `a_chat_turn_end_to_end_with_a_fake_claude` fails ("Cannot run claude"), so it is skipped via `checkFlags`.
  - The chat suite: `session::tests::a_cancel_logs_what_the_call_still_reports_and_what_it_never_took` cannot spawn its fake, its driver thread panics (`missing .../started`) and the test binary hangs. Also known flaky outside the sandbox (backlog). The whole chat suite is left to `devenv test`.
  - Both could run if the fakes used an interpreter path the sandbox has (`#!/bin/sh`, once checked POSIX-clean). That is a test change, not made here.
