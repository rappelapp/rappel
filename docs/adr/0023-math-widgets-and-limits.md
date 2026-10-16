# 23. Math widgets, limits and the licence gate

- Status: Accepted
- Date: 2026-10-16
- Amends: ADR 0021 point 3 (inline formulas add no widgets) and point 6 (the widget count, the 64 em starting point)

## Context

ADR 0021 chose RaTeX and left the widget design, the size cap and the `deny.toml` mechanics to the math slice. This is what the slice found and decided.

## Decision

1. **An inline formula is a child widget of its paragraph.** The paragraph (`MathParagraph`) is a custom widget. Pango lays the text out with an `AttrShape` box per formula, as in ADR 0021's "Inline B", and each formula is a `MathView` inside a horizontal `GtkScrolledWindow`, allocated where Pango put its box. The box is never wider than the line, so the text wraps at the card width and a formula that is too wide scrolls on its own. Drawing the formulas into the paragraph's own cairo node (the spike) would have left a wide formula only the choice of being clipped, and one accessible object for the whole paragraph. Paragraphs, headings and table cells with a drawn formula use it; text without one stays a `GtkLabel`.
2. **A drawn formula costs 2 widgets, inline or display** (the scroller and the view), against ADR 0021's "inline adds none". `widget_cost` charges a block for at most 200 formulas, the most a side draws, so a side costs at most 400 widgets for math by this count. That count is nominal: a `GtkScrolledWindow` also makes a viewport and two scrollbars, which I believe come to about 10 widgets per formula, and could not check here. The 200-formula cap is what bounds it (about 2,000 widgets on a side at worst); if the checklist shows the cost matters, wrap only formulas that are wider than their box.
3. **The size cap is 128 em on width and on height plus depth** (`math::MAX_EM`), one number for both. ADR 0021 asked for a test with a long derivation: 30 lines of `\frac` with a `\sum` and limits are 102 em tall, so 64 em would have refused a formula people write. 128 em is about 2,500 px wide at body size, which the scroller handles. `\rule{1000000em}{…}` and the like are far over.
4. **Errors.** Invalid LaTeX is shown as its source in monospace. In a block it is a label with the `error` style class. Inline, Pango markup cannot use a style class, so the source gets Pango's own wavy "error" underline, in the colour Pango picks (no colour of ours). A source over 4 KiB, a layout over the cap, and every formula after the 200th are plain source. All formulas count towards the 200, valid or not. RaTeX's messages can quote the card, so they go to the log at debug level only.
5. **Layout and cache.** A formula is laid out in the widget's current colour and cached (800 entries, enough for both sides of a card with 200 formulas each in two colours; each `MathView` also keeps its own list, so drawing never goes back to the cache; oldest dropped first, keyed by source, style and colour), including failures. A formula is laid out once in black to decide what to show, and once more in the theme colour when drawn; a light/dark switch lays each visible one out once more. A panic inside RaTeX would be a bug of its own, so it is caught and treated as a parse failure.
6. **Cost, measured** (release build, laying out and deciding, without widgets; 200 *distinct* sources, so the cache does not help):

   | 200 formulas of | Time |
   |---|---|
   | the largest `a_1+a_2+…` under the cap (240 bytes, 127 em wide) | 18 ms |
   | 3.8 KiB of empty groups `{}` (valid, narrow) | 118 ms |
   | 3.8 KiB of `\sqrt{x}`, `\frac{1}{2}`, `x_1`, `x` (laid out, then too wide) | 130–290 ms |
   | 3.8 KiB of `\,` (rejected as invalid) | 560 ms |

   Anything near 4 KiB is either too wide or rejected, so the cap, not the 4 KiB limit, is what bounds a valid side. Painting 200 formulas with their widgets was not measured (see the checklist of the PR).
7. **The "more is there" hint.** The scrollers have the `math-scroller` class. `GtkScrolledWindow` draws an `undershoot` node on the side with more content; Adwaita styles it only for tree and column views, so `data/style.css` gives `.math-scroller > undershoot.left/.right` a fade of the text colour. Whether it is visible enough at 360 px is for the maintainer to check.
8. **The licence gate** (ADR 0021 point 8):
   - `ratex-katex-fonts` gets a scoped `OFL-1.1` exception and a `clarify` entry, `MIT AND OFL-1.1`, pinned to `fonts/OFL.txt`.
   - **What happens when the pin stops matching** (tested by changing the hash, and by removing the exception): with a wrong hash, cargo-deny does not fail. It falls back to the crate's own `MIT` label and only warns "license exception was not encountered". With the exception removed, the check fails, as it should. So `scripts/check.sh` runs `cargo deny check -D license-exception-not-encountered`, which turns that warning into an error. A wrong hash now fails the gate, and so does a version bump whose `OFL.txt` changed.
   - The hash is cargo-deny's: xxHash32 (seed 0) of the file with `\r\n` replaced by `\n` (`src/lib.rs` `hash` and `licenses/gather.rs`, cargo-deny 0.20.2). It is not a CRC. After a `ratex-*` bump, recompute it for the new `OFL.txt` after reading the new `fonts/FONT_NOTICE.txt`. A release could add fonts under another licence and keep `OFL.txt`, which no tool here sees.
   - `RUSTSEC-2026-0192` is ignored with a reason and the review date 2027-01-16. The CHANGELOG repeats the date.
   - A copy of the OFL is in `LICENSES/KaTeX-fonts-OFL-1.1.txt` and the README credits the fonts. **The app has no About dialog yet**, so the ADR 0021 line "mention the fonts in the About dialog" waits for it. Add it there when About is built (SPEC §4.8).
9. **Accessibility.** Each `MathView` has the Math role and the LaTeX source as its label. The paragraph's own label is its text with each formula replaced by its source (the pure `accessible_text`), as a label would have read it before. A screen reader may therefore read an inline formula twice, once in the paragraph and once as an object. This could not be tried here; if it does, hide the views from the tree and keep the paragraph's label.

## Consequences

- Up to 400 more widgets for a side full of formulas. Painting that many was measured only for display widgets in the spike (321 ms for 200).
- The fade of the undershoot and the double reading are the two things most likely to need a follow-up after the maintainer's check.
- A drift guard test keeps `fill_scratch_db --math` and the app's test formulas the same.
- `\oiint` and `\mathbb{1}` remain as in ADR 0021, point 9.
