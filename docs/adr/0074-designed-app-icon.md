# 74. The designed app icon: index cards on a ring, drawn by the maintainer, and a symbolic icon to match

- Status: Accepted
- Date: 2026-11-03
- Amends: ADR 0060 (the icons are no longer placeholders; the icon test allows a reference to the file's own gradients)
- Refers to: SPEC §5 (publishing); GNOME HIG, app icons; Flathub's quality guidelines

## Context

ADR 0060 committed placeholder icons, to be replaced by a designed icon before 1.0. Flathub asks for an app icon drawn by a person, in a 128 × 128 frame, in the style of GNOME's own icons, with a 16 × 16 symbolic icon beside it. The maintainer went through eight rounds of concepts (a Leitner box, index cards, a card tin, a tuck box, ring-bound cards) and chose ring-bound index cards: a white card in front, a red one turned behind it, held by a simple grey ring. He then drew the icon himself in Figma.

The test of ADR 0060 refused any `url(` in an icon, to keep out links. The drawn icon fills its card edges and the red card's side with gradients, and SVG uses a gradient with `fill="url(#id)"`, a reference inside the same file.

## Decision

**App icon**: `data/icons/hicolor/scalable/apps/io.github.rsmirke.Rappel.svg` is the maintainer's drawing, exported from Figma at 128 × 128, unchanged except for the XML declaration and a comment at the top. A white index card (a red header rule, ruled lines and two handwritten words) stands in front of a red card turned about 11° behind it. Both cards have a thick bottom edge with light at its corners. A flat grey ring passes through a hole in the top-left corner. Nearly all the colours come from GNOME's palette (three near-misses, `#E01C25`, `#F3F3F3` and `#91908D`, are kept as drawn).

**Symbolic icon**: `data/icons/hicolor/symbolic/apps/io.github.rsmirke.Rappel-symbolic.svg` reduces the same idea to plain shapes on a 16 px grid: the front card as a 1.5 px outline with two text bars, the ring as a solid loop on its top-left corner with a 1 px gap around it, and the card behind as a strip at half opacity above it. It uses only filled paths, as GTK recolours symbolic icons by their fill.

**The icon test allows `url(#id)` when `id` is defined in the same file** (`urls_stay_in_file`, with its own unit test). The check ignores case, as CSS does (`URL(` is `url(`). Any other `url(…)`, for example to another file or with a space before `#`, still fails, as do `href`, `http` (other than SVG's namespace), scripts, pictures, style sheets, event attributes and, new here, any character reference (`&#`), which could spell `url(` or `http` in disguise. A reference to the file's own gradient loads nothing from outside, so the intent of ADR 0060, an icon made only of plain shapes, holds.

## Consequences

- The README, `ARCHITECTURE.md` and SPEC no longer call the icons placeholders.
- The symbolic icon was drawn from the concepts, not by the maintainer. Flathub's "drawn by a person" rule is about the app icon; the maintainer may still redraw the symbolic one.
- Nothing installs the icons yet. Packaging (phase 3b) installs them under `share/icons/hicolor/`. To see the icon on a desktop before then, copy it to `~/.local/share/icons/hicolor/scalable/apps/`.
