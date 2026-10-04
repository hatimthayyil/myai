# grep.rs

R5 of `ai/research/20261003T091838---memory-git-ref.md`; replaces `recall` (no alias).

- ripgrep conventions: Rust `regex`, `-F` literal, smart-case by default (`-i`, `-s`; the later flag wins, as in rg). `has_upper` skips escapes (`\S`, `\p{Greek}`, `\x{4A}`) so a class name does not turn case-sensitivity on; rg walks the regex AST, this is the cheap approximation.
- Matches the text field only: matching provenance would make `2026` or `claude` match everything. Provenance is filtered instead: `--since` (UTC midnight, compared as `ts` strings), `--origin`, `--agent`, `--session`, `--repo`; exact, ASCII case-insensitive. A summary is filtered on its own fields (who napped it, when); it has no repo, so `--repo` excludes every summary.
- `-t`: summaries too, printed `#lo-hi <text>` so a hit goes straight to `zoom`. Opt-in: summary hits duplicate leaf hits (RAPTOR: a deliberate second pass).
- Order: chronological by `key` = (last member, size). A summary sorts right after its last memory, smaller blocks first; keys are unique, so a cursor never splits a tie.
- Paging: newest page first, bounded by `PART_CHARS` and `-m`. One streaming pass (`log_scan`, one segment at a time), no index. Each source (log, each tree level) is already sorted, so each keeps its own newest page (`Page`), then the pages are merged and trimmed again. `floor` is the newest key any page dropped; the merge skips everything at or below it, so the printed page is a gap-free suffix of all hits. `--before <id>` shows keys strictly below the first printed id, so pages neither lose nor repeat a hit. The cursor is a position: stable until the next sync.
- A page always keeps its newest unit, even over budget, so paging always progresses.
- Footer `Newest K of N. Older: <cmd> --before <id>` reprints the user's flags (`command`): pattern always single-quoted, other values quoted only when needed.
- `-C N`: neighbouring memories, unfiltered (as grep). A unit is one hit with its before-context; after-context extends the newest unit. `--` separates units that are not adjacent; a summary is never adjacent. Context is log-only.
- `-c`: `N matches.`; zero hits is always `No match.`, exit 0: agents read nonzero as failure.
