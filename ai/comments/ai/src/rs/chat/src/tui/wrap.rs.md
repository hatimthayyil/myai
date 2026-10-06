# tui/wrap.rs

One greedy word wrap (break after spaces, split words wider than a row, grapheme widths from unicode-width) for both the composer (byte ranges, for cursor mapping) and history lines (styled spans sliced by those ranges). History is pre-wrapped so `Term` knows how many rows an insert takes.
