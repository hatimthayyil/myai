# Backlog

## Memory

- `ai chat` stages 1–2 implemented and smoke-tested against real Claude Code (sonnet; one prime + one turn). Pending: install the rebuilt CLI, re-import the old fixed-record live memory into a fresh store, measure real subscription usage (compaction especially) in the first week. Spec: `ai/research/20261004T194110---ai-chat.md`.
- `ai chat`: a pasted multi-line message is one turn only when its lines arrive together; a line editor with bracketed paste would make it exact.
- Multi-machine sync for the chat memory (rebase-style merge: pushed messages never move), deferred.
- Repo-level memory, alongside the user memory. Deferred. Decided (2026-10-04):
  - Purpose: portable with the repo, shared with collaborators, focus at wake. Inside a repo, repo memory has priority; user memory stays reachable.
  - Routing: every note goes to user memory. Notes that are repo-local and not personal also go to the repo memory, as the same record (same key).
  - Wake: both covers in one `WAKE_LINES` budget, repo first.
  - Sharing: one ref per person (`refs/ai/memory/<person>`) on the project remote; teammates' refs readable.
  - Open: who decides the repo copy (agent `note --repo` vs classifier, below); budget split; how teammates' refs appear (wake, grep/zoom, or not yet); creation and push policy; `<person>` identity; scope prefixes on ids (`u#`, `r#`).
- Classifier ("Jev type model") to pick the notes that also go to repo memory: repo-local and not personal. Deferred until repo memory exists.
- `ai chat`: test `session::tests::a_cancel_logs_what_the_call_still_reports_and_what_it_never_took` is flaky: failed once (2026-10-06) and the test binary then hung; passed 6 reruns. Find the race and make it deterministic.
