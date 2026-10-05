# claude.rs

Claude Code as the compactor's backend, flags from `ai/research/20261004T172045---shitty-optchat.md` §1/§4: `claude -p --model sonnet --effort medium`, stream-json in/out with `--verbose --include-partial-messages`, `--no-session-persistence --setting-sources "" --strict-mcp-config` (no hooks, plugins, CLAUDE.md, memory or user MCP leak into the request), `--system-prompt-file`, `--tools "" --safe-mode`; env `CLAUDE_CODE_PROMPT_CACHE_TTL=5m` (a 5 m mark after a 1 h one is a 400) and `DISABLE_PROMPT_CACHING=1` (our marks only).

- `compactor` resolves `claude` on `PATH` up front, so a machine without Claude Code fails at once instead of retrying forever. `compactor_at` takes the program (tests).
- System prompts are written once per text into a temp dir owned by the backend.
- One process per conversation: stdout lines go through a reader thread into a channel; the first `result` event ends a `say`. `is_error`, `stop_reason: refusal`, no result within 5 min, or an early exit (with stderr's tail) are errors. Drop kills the child.
- `command` and `Proc` also serve the chat master. `send` enqueues FIFO input on a dedicated writer thread, so a child that does not read stdin cannot block cancellation. `spawn_until` stops one-shot master/prime calls in the stdout reader at their terminal event; compactor conversations keep their process for size retries.
- Linux: own process group and PDEATHSIG SIGKILL. Drop sends group TERM, allows at most 200 ms, then kills the group and reaps the child synchronously. A stdout EOF from a living child cannot block `failure`. `Live::kill_all` atomically closes the registry against further spawns. Normal shutdown covers descendants in the group; parent SIGKILL guarantees the direct Claude child only.
