# record.rs

- `Kind`: user, talk, tool, echo, note (OptChat spec §2). `note` = `ai memory note` / `import`.
- `Message`: blob = one header line of space-separated fields (whitespace in a field becomes `_`, empty becomes `-`), `\n`, then the text verbatim (newlines kept: `zoom id+1` returns it whole). No size limit. `ts` UTC `YYYYMMDDThhmmssZ`; no ordering key (sync is gone).
- `label`: `kind: text`, what the tree compresses and what a free leaf holds.
- `Node`: one line `ts model text`. Text is stored flattened (`flat`): every consumer shows it flattened anyway (view, zoom, compactor merge step), so a free merge is `a + " " + b` instead of the spec's `a + "\n" + b`; same byte count.
- `stamp`: `date hh:mm repo` for grep lines.
- `time_in` renders a message's UTC timestamp in a supplied zone; MCP `date` uses the machine's local zone.
