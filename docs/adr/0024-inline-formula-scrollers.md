# 24. Inline formula scrollers without a scrollbar, and GTK in the tests

- Status: Accepted
- Date: 2026-10-16
- Amends: ADR 0023 point 1 (each formula is a `MathView` inside a horizontal `GtkScrolledWindow`)

## Context

Every inline formula narrower than about 46 px had a gap after it: `$x$` is 11 px wide, and the text after it started 46 px on. `MathParagraph` reserves a Pango box of the larger of the formula's width and its scroller's minimum width, so that the scroller never covers the text after it. The scroller's horizontal policy was `Automatic`. In that case GTK 4.24's `gtk_scrolled_window_measure` makes the minimum at least the scrollbar's: `minimum_req = MAX (minimum_req, min_scrollbar_width + sborder.left + sborder.right)`. It does that even while the bar is hidden, and even with overlay scrolling. It was not a unit or scale error: the Pango conversions are all in 1/1024 of a logical pixel.

No test caught it, because no test could start GTK: CI has no display.

## Decision

1. **An inline formula's scroller has the policy `External`** (`render::inline_math_scroller`). With `External`, GTK adds neither the scrollbar's minimum nor the child's, so the minimum is `min-content-width`, which is 0. The box Pango reserves is then the formula's width, capped at the line width as before. GTK source, 4.24:
   - `may_hscroll` is `hscrollbar_visible || policy == EXTERNAL`. The scroll and touch handlers check only that, so a formula wider than the line still scrolls (ADR 0021 point 3). It scrolls with a horizontal touchpad swipe, a tilt wheel, Shift+wheel (`scrolled_window_scroll` swaps the axes when Shift is held) or a touch drag. A plain wheel or a mouse drag does not scroll it: the drag and pan gestures are touch-only.
   - A vertical wheel event over a formula propagates (`scrolled_in_non_scrollable_dir`), so the page still scrolls.
   - The `undershoot` nodes are drawn from the adjustment, whatever the policy, so the fade from ADR 0023 point 7 still marks a formula that has more.
   - What is lost: a wide *inline* formula shows no overlay scrollbar while the pointer moves. The fade is the only hint, and with a plain mouse, Shift+wheel is the only way to scroll it. Display math keeps `Automatic` and its scrollbar: it has no text after it to push away.
2. **Tests may start GTK, and the gate needs a display.**
   - `math_paragraph::tests::the_box_for_an_inline_formula_is_as_wide_as_the_formula` builds real paragraphs. It asserts that the box Pango reserves is the formula's drawn width, within a pixel, at text scaling 1× and 2× (`gtk-xft-dpi` 96 and 192 dpi). It fails, not skips, without a display, so it cannot pass in CI without running.
   - CI installs Xvfb and runs `xvfb-run -a scripts/check.sh`. Locally the desktop session is the display, and no window is shown.
   - "Scale" here means text scaling. A HiDPI scale factor does not enter: formula sizes, Pango units and allocations are all in logical pixels.
   - `gtk::init` takes GLib's default main context for good (gtk4-rs 0.11 `rt.rs` acquires it and never releases it). It also refuses a second thread. The `Debounce` test iterates that same context. The test harness runs each test on its own thread, so the two crashed when run in parallel (a GLib hash-table assertion, then SIGSEGV), and failed when run one at a time. Tests that need the default main context or GTK now send their body to one thread that owns the context (`crate::testing::on_main_context` / `with_gtk`), and the bodies run one at a time. A panic in a body fails only its own test. A panic inside a GLib callback that the body runs aborts the whole test binary, because it cannot unwind through C.

## Consequences

- Short inline formulas sit in the text with normal spacing, at any text scale.
- The test fails on a headless machine without `xvfb-run`. README says so.
- If the missing scrollbar on wide inline formulas proves too subtle on the checklist, the next step is the one ADR 0023 point 2 mentions: wrap only the formulas wider than their box. That needs a layout pass to decide which ones those are.
