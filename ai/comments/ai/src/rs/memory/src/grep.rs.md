# grep.rs

- ripgrep conventions (smart case, `-F`, `-i`/`-s`). Matches message text only; filters `--since`, `--kind`, `--agent`, `--session`, `--repo`. `-t` adds nodes (levels ≥ 1; level-0 nodes duplicate messages); any message-only filter excludes nodes.
- Message lines `id+1 date hh:mm repo kind: text`, the text flattened and, past 400 bytes, cut to a window around the first match (`…` marks a cut). Search output, not the view: the agent zooms `id+1` for the whole message.
- Order by key `(last message, n)`; newest page first within `PART_CHARS` and `-m`; `--before <id+n>` pages back without loss or repeats. `floor` keeps the merged page a gap-free suffix of all hits.
