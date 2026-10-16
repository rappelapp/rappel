# Math rendering spike (branch `spike/math`, never merged)

Question: can RaTeX render Rappel's formulas well enough, inline and display, in GTK4?
Decision record: `docs/adr/0021-math-rendering.md` (separate `docs/adr-math` PR).

Both crates have their own `[workspace]`, so they are outside the Rappel workspace,
its `Cargo.lock` and `scripts/check.sh`.

- `math/`: path 1, RaTeX as published (`ratex-gtk4` needs gtk4 0.9, cairo 0.20).
  Inline A (TextView child anchors), inline B (Pango `AttrShape`, `src/inline.rs`),
  the 11 display formulas, invalid LaTeX, `--bench`, `--errors`, `--count N`, `--screenshot`.
- `math-gtk011/`: path 2, Rappel's versions (gtk4 0.11, cairo 0.22) with `ratex-cairo`
  bumped to cairo 0.22 (`math/vendor/ratex-cairo`, a one-line change: `version = "0.22"`).
- `math/screenshots/`: light/dark at 760 and 360 px, light at scale 2, path 2 light/dark.
  Taken offscreen through the Broadway backend and a `gsk::CairoRenderer`.

## Commands

    git switch spike/math
    cd spikes/math
    cargo run --release                  # header-bar moon button toggles dark mode
    cargo run --release -- --dark --width 360
    GDK_SCALE=2 cargo run --release      # HiDPI
    cargo run --release -- --count 50    # a card with 50 display formulas (timing on stderr)
    cargo run --release -- --bench       # headless parse/layout/render timings
    cargo run --release -- --errors      # invalid and hostile LaTeX, each in a child process
    cargo run --release -- --side        # worst cases for one 1 MiB card side
    cargo run --release -- --fallback    # first use of a character outside the KaTeX fonts
    cd ../math-gtk011 && cargo run --release [-- --dark]

    # Screenshots without a window on your desktop:
    gtk4-broadwayd :7 &
    GDK_BACKEND=broadway BROADWAY_DISPLAY=:7 cargo run --release -- --screenshot /tmp/shot.png
    GDK_BACKEND=broadway BROADWAY_DISPLAY=:7 SPIKE_SCALE=2 cargo run --release -- --screenshot /tmp/shot2.png
