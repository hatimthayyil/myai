# tui/cells.rs

History lines per event. User messages `› ` bold; unanswered ones dim with `· unanswered`. Tool calls one line: `• Name summary`, the summary being the telling argument (Bash command, file path, pattern, URL, query) else the JSON input, whitespace collapsed, 200 characters; `mcp__server__tool` shows as `server:tool`. Results: first 3 non-empty lines under `└`, `… +N lines`, red with `✗` when `is_error`. Control characters are stripped (`ui::plain`), tabs become 4 spaces.
