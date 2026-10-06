# ui/mod.rs

The seam between session logic and display. `Show` is everything the chat tells its user, as data, in stream order; `Render` consumes it. The session never formats output itself: `Plain` and `tui::Tui` decide the look. `Usage` formats the turn usage line (`N in · N read · N write · N out · S s`) for both.

Status-only events (`Model`, `Priming`, `Compacting`, `Pending`, `User`, `Unanswered`, `Turn`) are ignored by `Plain`, which keeps pipe output byte-identical to the pre-TUI printer (`session::tests::plain_output_of_a_turn_is_exact` pins it).
