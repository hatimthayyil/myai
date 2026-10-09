# meta.rs

The metadata a line shows between `id+n|` and its text, shared by `wake`, `zoom` and `grep` (one `#[command(flatten)]` `Meta`, so the flags and help are identical everywhere).

- Format: `id+n|dates|f1|f2|...|text`. One `|` per column, the text last, so a `|` in the text is harmless and the layout reads like `git log --format='%h|%ad|%s'`. The columns are fixed per invocation; agents parse it positionally.
- Dates always (the setup block's `id+n|dates|text`), UTC, from the messages the line covers, not the node's build time (`show` has that). `2026-10-04..10-05`: the end drops what it shares with the start (the year; with `--time`, the whole date when it is the same day: `2026-10-08 09:12..17:40`). A single day or minute is one value. ISO order so it sorts and greps; `..` is git's range notation. A space before the time (git `--date=iso`) reads better than `T` and is safe inside a `|` column. Second-resolution times are cut to minutes: enough to order events, 3 bytes cheaper.
- `--time`: a flag rather than a `--date=FORMAT` (git) or a `time` field: there is one useful variation, and the date column stays first and always present.
- `-o FIELD,...` (`ps -o`): origin, repo, head, branch, agent, model, session, kind, in the order given. A line over many messages shows one value: the commonest (a real value before `-`, ties to the earliest), then `(+k)` for the k other distinct values (`acme/widget(+2)`). Listing all values would grow without bound on old lines; a bare count would hide the main one. The agent zooms or `show`s for the rest.
- Cost: dates read only the first and last message (the log is in time order: store.rs.md); fields read every message the line covers. Plain `wake` reads 2 blobs per line; `wake -o` reads the whole log once. Simple per-message reads; a single scan would be faster on a large log, not needed yet.

`Meta::args` gives the flags as separate arguments for the usage log; `flags` joins them for the printed continuation commands. `date` is shared with stats' per-day rows.
