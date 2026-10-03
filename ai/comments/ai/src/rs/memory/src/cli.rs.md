# cli.rs

Ported from OptMem (github.com/VictorTaelin/OptMem). User-facing text names the tool `ai memory` and scopes memory to the repository.

- `Location::find`: default store is `<repo root>/.ai/memory` (see repo.rs); `repo` is set only then, so `init` touches `.gitignore` only for the default store.
- `Cli::run`: `init` is the only command that runs without an existing memory; every other command refuses, so a typo in `AI_MEMORY_DIR` is an error instead of a second, empty identity. Output goes through `out` so tests drive it in-process.
- `block_id`: parse `<lo>-<hi>` inclusive at both ends, and require a real block (aligned power of two). Without the shape check `4-5` and `5-6` read the same record.
- `paginate`: split the document into parts that survive any harness's output cap.
- `wake`: a part is rendered as of T, so a note landing between parts cannot shift a boundary and drop a line. The only reason to refuse is that the document cannot be written without a summary; work it does not need is handed over after the read. "Not awake yet. Run: ..." is the one instruction that survives every harness's truncation (pi drops the head), so it says both that the read is unfinished and how to continue. "You are awake." is always printed on the last part, even a one-part memory: the contract is "run parts until one says awake". After a refusal, if nothing is pending the record exists but is blank: point at `forget`.
- `nap`: resubmitting a settled block (two sessions paid the same nap) is not an error; a block neither settled nor next is.
- `config`: an empty value restores the default. Sizes only select what is printed, so changing one is free.
- `forget`: a summary can be wrong (mistyped, bad compression); drop it and everything above it.
- `recall`: one pass, keeping only the newest matches within `PART_CHARS`; a vague regex matches the whole log, which fits neither the output nor memory. Matches the whole line (id and date included).
- `zoom`: the agent navigates; the tool only reads. A half beyond T is the future and is omitted; an unbuilt half says "not compressed yet".
- `import`: bootstrap only. Refuse non-UTF-8 input and impossible dates (an impossible date would poison every later import's order check).
- `Cli::exec` / `broken_pipe`: a closed stdout (`| head`) is not an error; exit 0 silently, as ripgrep does. Checked once, over the whole anyhow chain.
