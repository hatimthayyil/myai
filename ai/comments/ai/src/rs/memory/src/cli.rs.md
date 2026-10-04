# cli.rs

Ported from OptMem (github.com/VictorTaelin/OptMem). User-facing text names the tool `ai memory`; the memory belongs to the user (one per machine), shared by every repo and session.

- `default_dir`: `$AI_MEMORY_DIR`, else `~/.ai/memory`. No repo discovery: the memory is per user.
- `Cli::run`: `init` is the only command that runs without an existing memory; every other command refuses, so a typo in `AI_MEMORY_DIR` is an error instead of a second, empty identity. Output goes through `out` so tests drive it in-process.
- `span`: parse any printed id, `#` optional: `<pos>` (one memory) or `<lo>-<hi>` inclusive at both ends, and require an aligned power of two. Without the shape check `4-5` and `5-6` read the same record. `block_id` (nap, forget) also requires size ≥ 2. `name` prints a span back as an id.
- `paginate`: split the document into parts that survive any harness's output cap.
- `wake`: a part is rendered as of T, so a note landing between parts cannot shift a boundary and drop a line. The only reason to refuse is that the document cannot be written without a summary; work it does not need is handed over after the read. "Not awake yet. Run: ..." is the one instruction that survives every harness's truncation (pi drops the head), so it says both that the read is unfinished and how to continue. "You are awake." is always printed on the last part, even a one-part memory: the contract is "run parts until one says awake". After a refusal, if nothing is pending the record exists but is blank: point at `forget`.
- `nap`: resubmitting a settled block (two sessions paid the same nap) is not an error; a block neither settled nor next is.
- `config`: an empty value restores the default. Sizes only select what is printed, so changing one is free.
- `forget`: a summary can be wrong (mistyped, bad compression); drop it and everything above it.
- `grep`: see grep.rs.
- `zoom`: R4 of the git-ref research note. The agent navigates; the tool only reads. `frontier` prints only the nodes `depth` levels down (default 3 = 8 lines; parents were already read). Raw shortcut: a block ≤ `RAW_MAX` (16) opens fully raw at any depth, and frontier nodes of size ≤ 2 print as their memories (a 2-summary is as long as its members). A node beyond T is the future and is omitted; an unbuilt (or blank) node says "not compressed yet", as before. Cap: 64 lines (`ZOOM_LINES`) and `PART_CHARS`; over it, refuse and name the deepest depth that fits. If no smaller depth fits (a raw block, a tiny `PART_CHARS`), print anyway: the agent must always be able to descend. Every printed line starts with an id `zoom` and `show` accept, a single position included.
- `show`: one record, every field, one per line. A block shows its summary's fields (`fp` included); an unbuilt block points at `zoom`.
- Line formats: `wake` and nap prompts `#<pos> <YYYY-MM-DD> <text>` (`Memory::line`); `zoom`, `grep` `#<pos> <YYYY-MM-DD hh:mm> <origin> <repo> <text>` (`Memory::detail`); summaries `#<lo-hi> <text>` everywhere.
- `import`: bootstrap only. Refuse non-UTF-8 input and impossible dates (an impossible date would poison every later import's order check).
- `Cli::exec` / `broken_pipe`: a closed stdout (`| head`) is not an error; exit 0 silently, as ripgrep does. Checked once, over the whole anyhow chain.
- Each read command works on one `Snapshot`; each mutation returns the snapshot it committed, and follow-up prompts (`note`'s nap, `nap`'s next) read that one.
- `note`: provenance from the cwd repo and env (see prov.rs). Warns when the assigned ts is over a day ahead of this clock: some clone's clock ran ahead and drags appends forward.
- `nap`: `@<hex>` is optional (hand-typed ids have none). Checked against the snapshot first, so a stale block that is no longer next still says "changed by a sync", then again inside the CAS. Hex is compared lowercase.
- `sync`: no remote is not an error: say so and how to add one. `report` renders a `sync::Report`; `verbose` (the `sync` command) adds "Pushed"/"Up to date" and the `Run: ai memory nap` tail. `wake` is terse: it prints its own nap prompt.
- `wake`: syncs first only on a fresh read (no `T`), so later parts of one read never renumber; a 3 s budget, any failure is one warning line and wake goes on.
- `import`: each memory wants its date's UTC midnight; the append rule spreads same-day memories by seconds. Commit message `import`.
