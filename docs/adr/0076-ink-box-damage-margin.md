# 76. A card side repaints a margin around its text (stray white dots, #120)

- Status: Accepted
- Date: 2026-11-09
- Refers to: issue #120; ADR 0019 (rendering card content), ADR 0021 (math); GTK 4.24 sources cited below

## Context

The maintainer saw light specks around a card's text after moving between cards, first next to inline math ("Mesure $\sigma$-finie"), then on plain text ("Tribu engendrée"), so math was not the cause. The issue's leading hypothesis was damage: GTK redraws only the parts of the window whose render nodes changed, and something drew outside the area GTK thought it covered.

### Evidence

**Reproduced under Xvfb** (`scripts/screenshot.py`, cairo renderer, dark, 1000 × 720): six due cards whose fronts are full of accents ("Ǻǻ Ŝŷ ĝĥ ÉÂÎ Ḟ ŰŐ", "Tribu engendrée", "Mesure *σ*-finie" …), each front and back screenshotted while studying, the same run done again with `GSK_DEBUG=full-redraw`, and the two compared pixel by pixel. Without the fix, two transitions differ from the full redraw in every run: one pixel row at the top of the stacked accents of "Ǻǻ" is missing when that card appears (14 × 1 px), and a faint dot is left to the right of "intégrale", where the previous card's accents were (51 × 9 px). With `full-redraw` there is neither. So it is damage.

**Why.** GTK's text node takes its bounds from the glyphs' ink extents *without hinting*: `gsk_text_node_new2` calls `gsk_get_glyph_string_extents` (gsk/gskprivate.c), documented as "like `pango_glyph_string_extents`, but it ignores hinting of the font". The glyphs are drawn *with* hinting: in the default font rendering mode ("automatic"), `gtk_widget_update_pango_context` (gtk/gtkwidget.c) gives every widget's Pango context `CAIRO_HINT_STYLE_SLIGHT`, which moves outlines vertically to the pixel grid. The damage is the old and the new nodes' bounds rounded out to whole pixels (`rectangle_init_from_graphene`, gsk/gskrendernode.c). A hinted accent can so be drawn one pixel row outside the damage: when it appears, that row is not painted, and when the card is replaced, that row keeps the old card's pixels: the dots. The unit test below measured it directly: in 31 of 124 drawings of an accented, italic string (10–40 px, four sub-pixel offsets), GTK's cairo renderer painted one row or column beyond the node's rounded-out bounds, never more.

This is in GTK, not in Rappel's own drawing: `MathView` draws into a cairo node whose bounds are its allocation (cairo nodes clip to their bounds), and `MathParagraph` draws its text with `append_layout`, as `GtkLabel` does. Neither draws outside what it tells GTK; the text node underneath does.

## Decision

**The box that holds a rendered card side, `InkBox` (`render/ink_box.rs`), adds a transparent colour node under its children, over their bounds grown by 2 px (`INK_MARGIN`), in a colour that differs at every snapshot.** `render::render` returns it, so the review page's front and back and the card editor's preview get it. A colour node whose colour or bounds changed is damaged as a whole (`gsk_color_node_diff`, gsk/gskcolornode.c), so each time the side draws anew, GTK repaints everything its glyphs can reach. The colour is fully transparent and draws nothing; its red, green and blue carry a 24-bit counter shared by all boxes, so a side that replaces another in the same place never matches it.

The margin is 2 logical px: the measured overflow is one device pixel, and a device pixel is at most one logical pixel. The test asserts that the grown bounds contain both the painted pixels and Pango's (hinted) ink extents, at every size and offset tried.

Alternatives not taken:

- **`GSK_DEBUG=full-redraw` or queueing a redraw of the window**: repaints the whole window every frame; the brief rules it out, and it would cost on every animation.
- **Turning hinting off for card text** (`gtk_widget_set_font_options` with no hinting): GTK ignores a widget's font options unless the user set font rendering to manual (the code above), so it would not apply by default.
- **Clipping the labels to their allocation**: clips the accents above the first line, and does not help, since the overflow is past the node's bounds, not past the allocation.
- **Drawing the text ourselves with corrected bounds**: would replace `GtkLabel` (wrapping, links, accessibility) for one pixel row.
- **Marking only a change of content** (a counter bumped by `fill`): misses changes inside the side that GTK redraws on its own (a hovered link's colour, the reveal's slide). The snapshot is called only when something in the side asked to be drawn, so marking every snapshot costs a repaint of the side's area exactly then, and nothing on frames where the side did not change.

## Consequences

- Moving between cards, revealing the answer and typing in the editor preview no longer leave dots or cut tops of accents, whatever the renderer, since the overflow comes from hinting, which every renderer applies.
- Whenever anything inside a side redraws (a hovered link, a playing sound's progress bar), the whole side's area is repainted, not just that widget. A side is small, and this happens only while something in it changes.
- Other text in the app (card rows in lists, deck rows, dialogs) can show the same one-pixel leftovers in principle. Nothing reported it, and those widgets are replaced or recycled with much larger damage around them (rows, pages); if it shows there, the same box can hold them.
- Not verified on the maintainer's renderer: under Xvfb, the GL and Vulkan renderers repaint the whole window every frame (no buffer age), so only the cairo renderer reproduces it there. The cause is in the text node's bounds, which all renderers share, so the fix applies to them too; the maintainer checks on his desktop.
- A single pixel at the right edge of an "e" stays 3 to 9 levels (out of 255) darker than with a full redraw, inside the repainted area, on the cairo renderer only. It is not damage (the area is repainted) and not visible.
- If GTK ever computes text node bounds from the hinted glyphs, `InkBox` can go; the test then shows nothing past the node's bounds.
