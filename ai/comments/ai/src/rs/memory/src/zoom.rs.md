# zoom.rs

Spec §7.1, shared by `ai memory zoom` and the chat's zoom tool. `n = 1`: `id+0|kind: text`, original newlines kept. Else the two children, `id+n/2|text`; `None` (rendered "No line id+n.") when the node itself is not built. `pages` cuts at a useful line end or UTF-8 boundary; CLI uses `PART_CHARS`, MCP caps page size to 29,000 bytes. `page` validates part numbers from 1; every byte remains reachable.
