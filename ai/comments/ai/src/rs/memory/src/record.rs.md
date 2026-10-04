# record.rs

Formats from R2 of `ai/research/20261003T091838---memory-git-ref.md`, with the addendum's `repo` field.

- `REC`: LOG and TREE records share one width, 512 B, so a record is found by slicing a segment.
- `LOG` widths: ts 16, origin 6, repo 32, head 12, branch 32, agent 32, model 32, session 36, then text 305. `TREE`: ts 16, origin 6, fp 16, agent 32, model 32, session 36, then text 367. Fields are space-separated and space-padded; the record ends in `\n`, so it stays one greppable line.
- `TEXT_MAX`: the smaller text room (LOG, 305): a memory must fit either record. Caps `ENTRY_CHARS`.
- `field`: meta fields are ASCII: whitespace → `_`, non-ASCII → `?`, truncated to width, empty → `-`.
- `decode`: slice bytes then decode each field; slicing decoded text would shift every boundary after a multi-byte character. Non-UTF-8 → corrupt. A log record without a 16-byte ASCII ts is corrupt too.
- Positions are never stored: a merge that inserts upstream would otherwise rewrite every later record.
- `ts`: UTC, `YYYYMMDDThhmmssZ`. `Memory::date` shows its UTC date; `line` (`wake`) prints `#<pos> <YYYY-MM-DD> <text>`, `detail` (`zoom`, `grep`) adds UTC `hh:mm`, origin and repo.
- `next_ts`: the append rule. `ts = max(want, tail.ts)`; if `(ts, origin) ≤ tail key`, `ts = tail.ts + 1s`. Keeps the log strictly sorted by key, so identical keys mean the identical memory and sync can merge by sorted union.
- `fingerprint`: first 64 bits of SHA-256 over the block's member keys, each `"ts origin\n"`. Lets a summary be validated against any log: sync reuses it at any offset, and the nap guard checks its first 4 hex.
- `crockford`: Crockford base32 alphabet (no I, L, O, U).
