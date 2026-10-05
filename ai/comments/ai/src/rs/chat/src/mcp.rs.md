# mcp.rs

Read-only newline JSON-RPC over stdio: initialize, ping, tools/list, tools/call. Notifications receive no response. zoom/date share the permanent-id memory store; each call reads a fresh snapshot. date renders local time. Long raw zoom results use PART_CHARS bounded further to 29,000 UTF-8 bytes plus continuation text; part numbers start at 1. Invalid ids and paging errors become tool results. Protocol and paging tests check that the ref remains unchanged.
