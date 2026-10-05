# Cargo.toml

Standalone chat crate, no workspace. Reuses memory storage/compaction/Claude process primitives; clap for CLI, signal-hook for terminal signals, jiff for local dates, serde_json for stream/MCP protocol. Fake scripts only in tests. Shared root target directory selected by CARGO_TARGET_DIR.
