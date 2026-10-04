# backend.rs

The pluggable model access the user asked for: `Backend::open(system)` starts a conversation, `Conversation::say(blocks)` sends one user message and returns the reply in the same context (size retries need that). A stateless HTTP API implements `Conversation` by keeping its own message history. `Block.cache` marks a cache breakpoint. Only implementation: `claude.rs`.
