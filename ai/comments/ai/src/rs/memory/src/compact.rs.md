# compact.rs

Spec §4. Library for `ai memory nap` now and `ai chat` (stage 2).

- Prompts: `prompts/compact.txt` is the spec's COMPACT verbatim with "OptChat" → "MyAI" (line breaks shift; whitespace only). `prompts/scale.txt` is shitty-optchat's hand-written 512-byte SCALE line.
- `message`: the step alone, no context and no ids. OptChat gave the compactor the whole view (up to 128 KB) so chat messages could be resolved; notes stand alone (OptMem's `nap` prompt also carries only the block), and the context made every call ~50k tokens.
- `summarize`: returns the call's `Who` (backend agent and model, the conversation's session) with the line; `put_node` records both. Trimmed, flattened reply; empty fails; over `NODE` → the cut-at-limit retry in the same conversation, up to `TRIES`; the shortest try wins (may stay a few bytes over: `NODE` is a target).
- `Compactor`: single-owner pump. The owning thread holds the `Store` and `Mem`; each job runs `summarize` on its own thread with an owned `Job` and sends the result back on a channel. Commit = `put_node` (one commit, with cascaded free parents), then the view refits. A failure is reported once per node (`take_reports`) and the node stays busy for `RETRY` (10 s, fixed, forever), then is pumped again. The job's context is snapshotted when the pump decides (rule 3 holds at that moment).
- Lock: `compact.lock` in the memory dir, `File::try_lock` (advisory, released by the OS when the process dies). `new` returns `None` when held.
- `run`: until nothing is due on the latest snapshot (re-absorbs notes from other processes before deciding it is done). `settle`: until every view line is built (stage 2's pre-turn wait). `log`: append messages and pump (stage 2).
- `refresh` absorbs external commits and pumps. `step` drains all completed jobs; `next_retry` bounds its wait. Dropping a compactor calls `Backend::stop`; Claude closes its registry and ends every running call, including concurrent opens. Detached worker results are discarded after shutdown.
