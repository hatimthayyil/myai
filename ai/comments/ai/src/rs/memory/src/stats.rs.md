# stats.rs

- `ai memory stats`: the name follows `atuin stats` (usage counts of a shell history) and `hledger stats`.
- Sources: notes from the store (every `Kind::Note` message, with its stored provenance; covers the time before the usage log existed), wake/zoom/grep/show from `usage.jsonl`. Log `note` lines only feed the `Failed:` line. Other commands (nap, config, import, stats) are logged but not counted.
- Counts successful calls only; failed ones are summed on `Failed:` (a refused note, a bad zoom id). `wake` counts every call, later parts (`wake 2 <commit>`) included.
- Default window: the last 14 UTC days, today included. `--since` as in grep.
- Rows: one per UTC day, or `--by agent|repo|session`, in order of first call; days without calls are left out. `all` totals the window.
- Sessions: `-` (no session id: a person at the terminal, an unknown harness) is left out. A session woke if it ran `wake` once in the window. Means (`Per woken session`) and shares (`Of these`, rounded half up) are over woken sessions only, counting their own calls. A session cut by the window start counts only its calls inside it.
- Not added: `--by model`, medians, JSON output. `usage.jsonl` is JSON Lines for anything else (`jq`).
