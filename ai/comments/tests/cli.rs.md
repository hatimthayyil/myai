# tests/cli.rs

Binary tests. `Sandbox` isolates every run: `HOME`/`XDG_CONFIG_HOME` in the temp dir, `GIT_CONFIG_NOSYSTEM`, agent/git identity env removed, and `AI_MEMORY_NAP=0` so a note never starts a background nap (a real `claude` on `PATH` would be called). Test git runs disable auto-maintenance (its `maintenance.lock` would race the "nothing written outside the store" listing).

`a_note_naps_in_the_background` turns the trigger on with a fake `claude` shell script first on `PATH`, answering every stream-json message with a fixed `result`: two 400-byte notes need a model merge, and the detached nap builds `0+2` within the deadline. Provenance, filters and `show` run here since only the binary reads cwd and env.
