# tui/markdown.rs

pulldown-cmark (strikethrough, task lists; no tables: table source shows as typed) to styled `Line`s. Headings bold (H1/H2 cyan, H1 underlined, `#` dropped), bold/italic/strikethrough, inline code cyan, links underlined with ` (url)`, `•`/`N.` bullets with hanging indent, `│ ` quotes, fences dropped and code highlighted by syntect (default syntaxes, `base16-ocean.dark`, pure-Rust `regex-fancy`; RGB colours assume a dark truecolor terminal), a rule as 32 `─`. Blank line between top-level blocks.

Soft breaks are line breaks: a line once shown never changes while the reply streams.

`Rendered::last` is where the last top-level block starts (byte, line), for `Stream` to stop re-rendering settled blocks. `Code` caches the highlighted lines of the block last seen, so a growing code block highlights each line once (re-highlighting it per new line is quadratic).
