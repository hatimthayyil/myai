# zoom.rs

Spec §7.1, shared by `ai memory zoom` and (stage 2) the chat's zoom tool. `n = 1`: `id+0|kind: text`, whole, newlines kept, no CAP (spec; open question). Else the two children, `id+n/2|text`; `None` (rendered "No line id+n.") when the node itself is not built.
