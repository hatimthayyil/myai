# record.rs

- `Kind`: user, ai, tool, note. Spec §2 has `talk` and `echo`; here a reply is `ai` and a tool call and its result are one `tool` message. A tool message's leaf is empty (`Kind::shown`, `Message::line`): the view and compactor context skip empty lines, a merge with an empty side is the other side verbatim (no model call), zoom shows it as `HIDDEN`, `zoom id+1` and grep still give it whole. `note` = `ai memory note` / `import`.
- `Message`: blob = one header line of space-separated fields (whitespace in a field becomes `_`, empty becomes `-`), `\n`, then the text verbatim (newlines kept: `zoom id+1` returns it whole). No size limit. `ts` UTC `YYYYMMDDThhmmssZ`; no ordering key (sync is gone).
- `label`: `kind: text`, what the tree compresses and what a free leaf holds.
- `Message.origin`: the writing store's id; empty in `Message::new`, filled by `Store::append` (a given origin is kept, so a rebuild can carry old ones).
- `Node`: one line `ts origin agent model session text`; the text is last and alone may hold spaces, so `splitn(6)` parses it. Text is stored flattened (`flat`): every consumer shows it flattened anyway (view, zoom, compactor merge step), so a free merge is `a + " " + b` instead of the spec's `a + "\n" + b`; same byte count.
- `stamp`: `date hh:mm origin repo` for grep lines.
- `time_in` renders a message's UTC timestamp in a supplied zone; MCP `date` uses the machine's local zone.
