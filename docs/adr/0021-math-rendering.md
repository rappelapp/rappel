# 21. Math rendering with RaTeX

- Status: Accepted
- Date: 2026-10-16
- Accepted: 2026-10-16, by the maintainer
- Amends: SPEC §4.2 (inline math is drawn as vectors in the text flow, not as a texture), SPEC §1 principle 5 (reading system fonts, see point 7)

## Context

Cards are Markdown with LaTeX math (SPEC §4.2). Since ADR 0019, `$…$` and `$$…$$` are shown as monospace source. SPEC says to evaluate RaTeX first (pure Rust, KaTeX-compatible), with monospace source as the fallback and never a web view (§1 principle 3, §4.2).

A timeboxed spike on branch `spike/math` (never merged; `spikes/README.md` there has the commands) rendered 16 test formulas from real course material: 5 inline, in 4 French sentences (sentence 1 has two), and 11 display. It used RaTeX 0.1.15 in two ways:

- **Path 1:** `ratex-gtk4` / `ratex-cairo` as published, on gtk4 0.9. Everything was tested here: inline, display, invalid input.
- **Path 2:** Rappel's versions (gtk4 0.11, cairo 0.22), with `ratex-cairo` bumped from cairo-rs 0.20 to 0.22. Only the inline widget was exercised on path 2. Display formulas use the same `ratex-cairo` drawing call, but no display widget was built on path 2.

Screenshots in light and dark mode, at 760 px and 360 px wide, and at scale 2 are in `spikes/math/screenshots/` on that branch. Repository figures below were collected on 2026-10-16.

### Results

**Formulas.** 14 correct, 1 limitation that KaTeX shares (#11), 1 wrong (#13).

| # | Result |
|---|---|
| 1a, 1b, 2–4 (inline) | Correct: `\mathcal`, `\mathbb{R}`, `\lVert…\rVert`, `\sup_{…}`, `\limsup`, `\overrightarrow`, `\vec`, `\wedge`, `\underset`, `\sqrt`, `\left(\right)` |
| 5, 6, 7, 9, 12 | Correct: integrals and sums with limits, `\mathrm{d}`, `\text{em}`, `\left|`, fractional exponents |
| 8 | Correct: four-column `aligned`, `\overrightarrow{\operatorname{rot}}` |
| 10 | Correct: `vmatrix` with `\cdots \vdots \ddots`, `\prod` with a two-level subscript |
| 11 | KaTeX limitation: `\mathbb{1}` is a plain upright 1, because KaTeX's blackboard font has capital letters only (LaTeX needs `bbm` or `dsfont`). Not what the author meant. `cases` and `\text{si }` are correct |
| 13 | **Wrong:** `\oiint` draws the ring and the two integral signs on top of each other, which looks like a blot. `\oint`, `\mathcal{C}` and `\text{enlacé}` (accent inside `\text`) are correct |
| 14, 15 | Correct: `\cfrac` (three levels), `\hbar`, `pmatrix`, `\binom`, `\exp` |

**Invalid and hostile input.** Nothing crashed or hung. Each case ran in a child process on its main thread, as the app would run it.
- These give a clean `Err`:
  - `\frac{`, `x^`, `}}}`, an unclosed `\left(` and an unclosed `aligned` give a parse error with its position.
  - An unknown command (`\notacommand`, `\includegraphics{/etc/passwd}`) gives "Undefined control sequence". No file is read.
  - Deep nesting (1 MiB of `{`, 10 000 nested `\frac`, 2 000 nested braces, 200 nested `\sqrt`) gives "Recursion limit exceeded".
  - 100 000 characters give "Layout resource limit exceeded".
- Each rejection costs 0.1 ms or less in-process (0.01–0.02 ms in the side test below). The 100 000-character case costs 34 ms.
- Three inputs are accepted and need rules on our side:
  - `\rule{1000000em}{1000000em}` lays out a formula of 19 million × 19 million px.
  - `\href{…}{x}` and `\url{…}` render their text. RaTeX contains no network code: no `std::net` and no HTTP crate in the tree.

**Cost of a whole 1 MiB side** (`--side`, parse, layout and cairo render on one thread, no widgets).

| Side filled with | Formulas | Time |
|---|---|---|
| 2 000-brace formulas (all rejected) | 261 | 2 ms |
| 200-deep `\sqrt` (all rejected) | 746 | 13 ms |
| 100 000-character formulas (all rejected) | 10 | 0.28 s |
| `$x$` | 262 144 | **3.2 s** |
| 20-deep `x^{x^{…}}` | 12 483 | 1.25 s |
| formula #15 | 5 667 | 1.07 s |

Hostile formulas are cheap. Many small *valid* formulas are what can freeze the app, so a count cap is needed (point 6).

**Visual quality.**
- Glyphs are drawn as cairo paths from the outlines of the KaTeX fonts, with `Antialias::Best`. They are smooth at scale 1 and stay sharp at scale 2, because the shapes are vectors rasterised at the device scale.
- Inline math is 1.21 × the body size, as in KaTeX.
- Inline baseline:
  - **Inline A** (TextView child anchors, path 1): the formula's bottom edge sits on the text baseline, so formulas with a depth (subscripts, `\frac`) float above the line. Rejected.
  - **Inline B** (one widget per paragraph): Pango lays out the text with an `AttrShape` box for each formula. The box's logical rectangle starts `baseline` above the line, so the formula's baseline lands exactly on the text baseline. The formula is drawn into a cairo node at Pango's position. Wrapping works, and tall formulas push their line apart, as in a browser.
- At 360 px:
  - Display formulas wider than the card (#7, #15) scroll horizontally. Overlay scrollbars are hidden until the pointer moves, so the formulas look clipped, with no sign that more is there.
  - Pango cannot break inside an `AttrShape`. An inline formula wider than the line overflows it. The spike did not handle this.

**Dark mode.** The colour comes from the widget's CSS `color` (theme foreground) when the snapshot is drawn. It follows light and dark live in both paths. An explicit `\textcolor` keeps its colour.

**Speed.** Release build. GTK 4.24.1, Broadway backend and a software renderer.
- Card of 50 formulas, headless:
  - Parse and layout: 2.5 ms in total.
  - Cairo rendering at scale 2: about 21 ms in total.
  - Per formula: median 0.4 ms, slowest 0.9 ms.
  - The first pass costs only 10 % more, because fonts load lazily and are cached.
- In the app:
  - 50 display formulas: widgets built in 7 ms, first frame painted 89 ms after `present()`.
  - 200 display formulas: first frame painted 321 ms after `present()`.
  - All formulas were on screen, because the screenshot mode does not scroll.

**System fonts.** `ratex-cairo` always depends on `ratex-unicode-font`. A character that is not in the KaTeX fonts (CJK, emoji) makes it load a system font:
- It tries built-in paths, then runs `fontdb`'s system font scan, and reads the whole font file.
- First use costs 11 ms for CJK (Noto Sans CJK) and 6 ms for emoji, on the calling thread.
- The `RATEX_UNICODE_FONT` environment variable can name any font file to use instead.
- It writes one line to stderr with `eprintln!` (`[ratex-unicode-font] found via …`), bypassing `tracing`.

**Maturity, licence, size.**
- **Project:** github.com/erweixin/RaTeX, MIT.
  - Created 2026-03-10. 0.1.15 was released on 2026-10-16. Earlier releases: 0.1.12 in June, 0.1.13 and 0.1.14 in July.
  - About 1.5 k stars and 16 contributors, but one author wrote about 80 % of the commits.
  - 67 commits from mid-July to 2026-10-16.
  - 11 open issues (3 bugs, none about these formulas) and 2 open PRs.
- **gtk-rs versions:**
  - `ratex-gtk4` needs gtk4 **0.9** and `ratex-cairo` needs cairo-rs **0.20**, while Rappel is on 0.11 / 0.22.
  - Both `cairo-sys-rs` versions declare `links = "cairo"`, so Cargo refuses to put both in one build.
  - The layout crates (`ratex-parser`, `ratex-layout`, `ratex-types`, `ratex-font*`) do not depend on gtk or cairo.
  - Bumping `ratex-cairo` to cairo-rs 0.22 is a one-line change. It compiles, its own tests pass, and path 2 renders the inline sentences exactly like path 1, in light and dark.
- **cargo-deny with Rappel's `deny.toml`:**
  - Licences, bans and sources pass.
  - Advisories **fail**: `ttf-parser` 0.25 is marked unmaintained (RUSTSEC-2026-0192). It is not a vulnerability, there is no fixed version, and the advisory suggests `skrifa`. It comes in through `ab_glyph` and `fontdb`.
- **Font licence:**
  - `ratex-katex-fonts` is labelled MIT, but its 20 embedded `KaTeX_*.ttf` (514 kB) are **OFL-1.1**, per its `FONT_NOTICE.txt` and `OFL.txt`. cargo-deny cannot see this.
  - OFL-1.1 is not in our allow-list. `deny.toml` admits only licences that can be combined into a GPL-3.0-or-later program, checked against the FSF list.
  - The FSF lists the OFL as a free copyleft licence for fonts. To my understanding, it is *not* listed as GPL-compatible. I could not load gnu.org during the spike, so the maintainer should check the entry.
  - The case for shipping the fonts is that they are separate data, aggregated with the program, not code combined with it. This is the usual case for OFL fonts in GPL applications.
- **Size** (measured against a bare libadwaita app, not Rappel):
  - A stripped release build of a bare libadwaita window is 0.35 MB, and path 2 is 5.5 MB, so RaTeX adds up to **5.2 MB**. The fonts are 0.5 MB of that, and the rest is mostly the parser and its tables, regex, fontdb and rust-embed. In Rappel the increase will be somewhat smaller, because some of these crates are already in Rappel's tree.
  - Compared with Rappel's `Cargo.lock`, path 2 adds 60 packages. `serde` and `serde_json` are already in Rappel.

### Alternatives (only briefly: RaTeX did not fall short)

- **ReX** (pure Rust TeX layout, github.com/ReTeX/ReX): last push July 2020, 19 open issues. Not maintained.
- **MicroTeX** (C++, has a cairo backend): there is no Rust crate. We would have to write and maintain FFI bindings.
- **`katex` and `mathjax_svg` crates:** they embed a JavaScript engine and output HTML or SVG. SPEC forbids rendering HTML (§1 principle 5), and CLAUDE.md and §4.3 forbid SVG.
- **`pulldown-latex` and `latex2mathml`:** they output MathML, and GTK cannot render MathML.
- **Monospace source** (today): always correct and readable for short formulas, unreadable for #8, #10 and #15. It stays as the error fallback.

## Decision

1. **Use RaTeX's layout crates plus `ratex-cairo`, not the `ratex-gtk4` widget.**
   - `ratex-gtk4` is tied to gtk4 0.9.
   - It lays out every formula twice: once in black, then again in the theme colour.
   - It exports a C ABI we do not need.
   - Our widgets draw through `Snapshot::append_cairo`.
2. **cairo-rs 0.22.** We do not contact upstream for now. When `feat/math` starts, `ratex-cairo` 0.1.15 is copied into `third_party/ratex-cairo` with the tested bump (`cairo-rs` 0.20 to 0.22). The spike's copy in `spikes/math/vendor/ratex-cairo` on `spike/math` is the tested one, and the math slice re-runs its tests. It is used through `[patch.crates-io]` with a path (not git, because `deny.toml` has `allow-git = []`). Keep its MIT `LICENSE` and add a short note in the directory saying what was changed and why. Remove the directory and the patch once upstream publishes a `ratex-cairo` release on our gtk-rs versions, checked at every `ratex-*` bump. Writing our own painter is not planned.
   - Note for the slice: a path dependency inside the workspace directory becomes a workspace member automatically. The math slice adds `exclude = ["third_party"]` (or equivalent) and checks that `scripts/check.sh` does not run fmt, clippy or tests on vendored code.
3. **Inline math: one widget per paragraph**, as in inline B. Text plus `AttrShape` boxes with each formula's height and depth, then the formulas drawn at Pango's positions. Formulas are vectors drawn at the device scale, not textures. Body text keeps Pango markup from ADR 0019. Math is 1.21 × the body size. If an inline formula is wider than the line, the text still wraps at the card width. Only that formula overflows, and it scrolls horizontally like display math; the math slice decides whether this is done by laying the formula out as its own block.
4. **Display math:** a centred block that scrolls horizontally when wider than the card.
   - Like ADR 0019's tables, the scroller cannot take focus, so it scrolls by pointer and touch only.
   - The math slice must show that more is there. GTK's `undershoot` style classes on the scroller are the first thing to try. To be checked at 360 px.
5. **Colour:** the widget's CSS `color`, read when the snapshot is drawn. Display lists are cached per colour and laid out again when it changes.
6. **Errors and limits:**
   - A parse error shows the source in monospace with the `error` style (SPEC §4.2).
   - A formula whose source is longer than 4 KiB is shown as source without being parsed. This bounds the 34 ms case.
   - A formula whose laid-out width or height is above a cap is shown as source, which covers `\rule{1e6em}{…}`. The cap applies to both width and height. 64 em is a starting point, but a long `aligned` derivation of 30 lines is about 75 em tall, so the math slice tests one before fixing the number.
   - At most **200 formulas per side** are rendered. Later ones stay monospace source. For the test formulas (≤ 0.9 ms each), that is about 0.2 s of layout and drawing, instead of 3.2 s, plus the widgets (200 display formulas painted their first frame in 321 ms). Formulas up to the 4 KiB cap were not measured, so the math slice measures 200 valid formulas of nearly 4 KiB with widgets.
   - A display formula counts as 2 widgets in ADR 0019's 2,000-widget budget (scroller and drawing area). Inline formulas add none, because they are drawn inside their paragraph's widget.
   - `\href` and `\url` stay plain text, never clickable.
   - The screen-reader text stays the LaTeX source (`plain_text`, ADR 0019).
7. **System fonts (SPEC §1 principle 5).** The principle said card text "never triggers network or file access outside the app's data directory". With RaTeX, a CJK character or an emoji inside `$…$` reads a system font file. Pango already does the same for the same characters in ordinary card text, through fontconfig. **Decided:** the principle now reads "Card content never causes network access or access to user files. Reading installed system fonts is allowed." Two side effects are accepted:
   - `RATEX_UNICODE_FONT`: a variable only the user can set.
   - The one-line stderr message on first fallback, to report upstream later (go through `log`, or be opt-in).

   The rejected alternative was to show a formula containing such characters as source, which hurts cards in Chinese or Japanese.
8. **Gate changes.** Both decided by the maintainer. They change `deny.toml`, which CLAUDE.md protects, so each states its reason. They are made in the math slice, together with the dependency, because cargo-deny can only check them once `ratex-katex-fonts` is in the tree. This ADR PR changes neither `deny.toml` nor the README.
   - **OFL-1.1:**
     - A *scoped* `[[licenses.exceptions]]` that allows `OFL-1.1` for `ratex-katex-fonts` only, not a global `allow`, with a comment saying why: the crate embeds the KaTeX fonts, which are OFL-1.1 data aggregated with the program, not code combined with it.
     - A `[[licenses.clarify]]` that records the crate as `MIT AND OFL-1.1`, pinned by the hash of its `fonts/OFL.txt` (`license-files`). This alone does not guarantee that a licence change fails the gate: a release could add fonts under another licence and keep `OFL.txt`, and a new crate version may fall back to its `MIT` metadata. So the math slice checks how cargo-deny behaves when the hash or the version no longer matches, and every `ratex-*` bump includes a manual review of `FONT_NOTICE.txt`.
     - Ship `OFL.txt` with the app, mention the fonts in the About dialog, and **credit the KaTeX fonts in the README**.
     - Basis: the fonts are separate data, not code combined with the GPL program. This does not rest on an FSF compatibility listing, which, as far as I know, the OFL does not have.
   - **RUSTSEC-2026-0192:** an `ignore` entry in `deny.toml` with a reason: unmaintained, not a vulnerability; `ttf-parser` only parses the bundled KaTeX fonts and installed system fonts. **Review date: 2027-01-16** (three months). cargo-deny has no expiry field, so the date is written in the `reason`, and the math slice adds a reminder (CHANGELOG or issue). Review it earlier whenever RaTeX, ab_glyph or fontdb update, and remove it when RaTeX moves to `skrifa`.
9. **Known limitation and upstream reports:** `\oiint` renders wrongly (the ring and the two integral signs overlap, formula #13). This is a known limitation of RaTeX 0.1.15. It is not reported upstream for now, nor is the stderr output. To report or work around later, for example in the vendored crate or by showing such a formula as source. `\mathbb{1}` matches KaTeX, so card authors can type 𝟙 (U+1D7D9) instead.

## Consequences

- The cards' real formulas render like KaTeX, inline on the text baseline and as display blocks, and follow the light and dark themes. Invalid, oversized or excess formulas fall back to the current monospace source.
- The app grows by up to 5 MB and about 60 packages (RaTeX, ab_glyph, fontdb, regex, rust-embed…), plus bundled OFL fonts.
- We depend on a young project that mostly has one author. The layout crates do not depend on GTK, and the drawing step is small, so a fork or our own painter is a bounded fallback.
- `\oiint` renders wrongly until it is reported or worked around.
- We carry a vendored `ratex-cairo` until upstream moves; the vendored copy is ours to keep working.
- SPEC §4.2 ("render inline math to a texture") and §1 principle 5 are amended when this ADR is accepted.
- The `ttf-parser` ignore is temporary: review it by 2027-01-16, and when RaTeX drops `ttf-parser`.
- Untested so far: the display widget on path 2, a real HiDPI monitor with the GL renderer (the spike's scale-2 shots used the cairo renderer), and screen readers.
