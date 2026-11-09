# Phase 3b: brief for batch mode (target: v0.4.0 and the first Snap)

*Lead engineer, 9 November 2026. Draft for the maintainer's approval. When approved, slice 0 commits it as `docs/briefs/phase-3b.md`.*

## In short

This batch polishes everything found since 0.4.0-beta.1 and prepares the first Snap. It fixes the white dots (#120) and renames the app ID to `io.github.rappelapp.Rappel` (#130). It redesigns the card lists (#129 and #128, on the deck page and All Cards) and the Statistics page (design "D, a month at a time"). It adds the automatic backup folder (#127). Then it packages Rappel as a strictly confined snap that CI builds, finishes the store listing, and ends with version **0.4.0**, the first stable release, as a draft PR `phase-3b` → `main`. After it, the maintainer merges, tags, installs the snap from CI, tests it and uploads it to the Snap Store.

The Flatpak bundle, the translation workflow for volunteers, and 1.0 move to a later phase (3c).

## Standards and setup

Phases 1 to 3a set the standards, and they still apply: GNOME HIG, libadwaita docs checked, and every slice gated by `scripts/check.sh` under Xvfb and reviewed by the `reviewer` agent until nothing is blocking. Screenshots of every changed screen: light, dark, 1000 px, 360×640, and French at 360×640 for new strings.

- Repository: **rappelapp/rappel** (moved 5 Nov). Run `git remote -v` first; if it still says rsmirke, run `git remote set-url origin https://github.com/rappelapp/rappel.git`.
- Integration branch: `phase-3b`, from `main` at `57698be` (icon #126) or later. The maintainer creates it before the batch starts.
- Repo rules learned in 3a: merge commits are refused (squash or rebase only). Wait for CI with `gh pr checks <n> --watch`, never `--json` polling.

### Models (spend Opus only where it pays)

| Who | Model | Why |
|---|---|---|
| The orchestrator (the main session) | **Sonnet** (`claude --model sonnet`) | It reads the brief, starts sub-agents, watches CI and merges. It writes no logic. |
| Implementer, slices 0, 2, 6, 8, and 10's version bump | **Sonnet** | Mechanical work: docs, renames, metadata, version bumps. |
| Final report (slice 10) | **Opus** sub-agent | The report is the maintainer's only view of the batch, so it needs judgement. |
| Implementer, slices 1, 3, 4, 5, 7, 9 | **Opus** | Render bug, custom drawing, list performance, file deletion rules, packaging, attacks. |
| Reviewer | **Opus** (fixed in `.claude/agents/reviewer.md`) | The reviewer is how quality is checked, so it stays strong. |

Escalation: if a Sonnet slice still has blocking findings after its **second** review round, an Opus implementer takes over the same branch, keeping verified work (as in 3a, slice 7). Note it in the report.

Token savings that do not cost quality:
- Each implementer gets only its slice text, plus the files it names, not the whole brief.
- The reviewer reviews the slice's diff (`git diff phase-3b...HEAD`), not the repo.
- Screenshots only of screens the slice changed.
- CI runs once per PR (CLAUDE.md "CI budget").

### Pre-approved (not stop conditions)
- **#127 backups:** a new setting (the backup folder) stored in a key file in the data directory, like `window-state.ini` (ADR 0039); no migration. The backup job **may delete files it wrote itself**, and nothing else: regular files (not symlinks) in that folder whose names match its own exact pattern, beyond the 14 newest.
- **#130 rename:** renaming files and IDs from `io.github.rsmirke.Rappel` to `io.github.rappelapp.Rappel`. The data directory does **not** change (`~/.local/share/rappel/`).
- New files for packaging: `snap/snapcraft.yaml`, a CI workflow that builds the snap, and install rules for data files.
- New dependencies whose licence `deny.toml` already allows (none is expected).

### Stop conditions (write in `QUESTIONS.md`, skip to the next independent slice)
- Any migration or export-format change.
- Any deletion of user data other than the backup rule above.
- **Slice 7:** if the snap cannot give libadwaita ≥ 1.8 and GtkSourceView 5, or if glycin's image sandbox (bubblewrap) does not run under strict confinement. Weakening the image sandbox is a security decision for the maintainer.
- Any snap plug beyond the standard desktop set (see slice 7), and classic confinement in particular.

## Slices, in order

**0. Housekeeping** (docs only, Sonnet).
- Commit this brief as `docs/briefs/phase-3b.md` and the two mockups in `docs/briefs/mockups/` (the maintainer saves them there before the batch).
- CLAUDE.md: the status line, and batch mode's two names (`docs/briefs/phase-3b.md`, `phase-3b`); also app ID and repo lines once slice 2 lands (do it in slice 2).
- SPEC status line and §4/§5: Phase 3b (polish and Snap) ends in 0.4.0 on the Snap Store; Phase 3c (Flatpak bundle, translation workflow, hardening) ends in 1.0.
- `QUESTIONS.md`: record the maintainer's answers to Phase 3a questions 1 to 7, all "as recommended" (3 Nov).
- Amend `docs/briefs/phase-3a.md` for the record: #117 was folded into slice 7, and slice 7b (#119, duplicate-front warning) was added.
- CHANGELOG "Unreleased".

**1. #120: stray white dots** (Opus). Light specks appear around text after moving between cards, also on plain text ("Tribu engendrée"), so math is not the cause. The leading hypothesis is that something draws outside its allocation, such as accents or italic overhang (Pango's ink rect wider than its logical rect) or a custom snapshot without a clip. GTK then doesn't repaint that strip when the next card's damage is smaller, and leftovers of the previous card stay. Steps:
- Reproduce under Xvfb: two cards with diacritics and formulas, switch between them, screenshot.
- Confirm with `GSK_DEBUG=full-redraw`: if the dots vanish, it is damage.
- Fix at the cause: correct measure or clip in the custom widgets (`render/math_view.rs`, `render/math_paragraph.rs`, `widgets/card_row.rs` and anything else that snapshots). A blanket full redraw is not a fix.
- A test: a pure function that computes the drawn bounds, checked to contain Pango's ink extents for a string with accents and italics. If the cause turns out to be elsewhere, follow the evidence and record it in an ADR.

**2. #130: app ID and identity rename** (Sonnet).
- `io.github.rsmirke.Rappel` becomes `io.github.rappelapp.Rappel` everywhere: the files in `data/` (desktop, metainfo, both icons), GResource paths `/io/github/rappelapp/Rappel/`, the application ID, `po/POTFILES.in`, and tests.
- Every URL points to `https://github.com/rappelapp/rappel`.
- The developer name is **"The Rappel Project"** in About and the metainfo, with developer id `io.github.rappelapp`. Remove the personal name from `dialogs/about.rs` and from the icon SVG's comment ("drawn by the maintainer"). Leave historical ADRs and the CHANGELOG as written, and leave `translator-credits` alone.
- Note that the data directory does not change, and say so in the CHANGELOG.
- ADR 0075.
- The report tells the maintainer how to remove the old desktop file and icons from `~/.local/share` after installing.

**3. Card lists, proposition A** (#129, #128; Opus). Mockup: `docs/briefs/mockups/deck-page-mockups.html` (proposition A).
- One shared row widget for the deck page and All Cards, built on a boxed-list look inside a `GtkListView`, so 10,000 cards stay smooth.
- Two-line rows: the front, then the back or the subtitle.
- Tags shared by every listed card appear once above the list. Each row shows only its other tags, with "+N".
- All Cards adaptation: tags appear once above the list only when every card on screen shares them, which usually happens after filtering by deck or tag. Otherwise each row shows its own tags. The deck name and due date go on the subtitle line.
- #128: math in the front and back summaries is drawn ("σ", not "\sigma"), using the existing renderer at row size and kept on one line. Broken LaTeX falls back to its plain text.
- The pure decisions get unit tests: which tags are shared, "+N", and the subtitle text.

**4. Statistics page, design D "a month at a time"** (Opus). Mockup: section `#d-month` of `docs/briefs/mockups/stats-mockups.html` (drawn by the function `E()`).
- One boxed card, about 600 px wide, at the top of the clamp, with an `AdwToggleGroup` switch (libadwaita ≥ 1.7): Reviews | Time | Forecast.
- Headline numbers laid out like GNOME Wellbeing:
  - Reviews: "Today N", with "N failed" under it, and "Last 30 Days N". The Passed and Failed legend sits at the top right, on Reviews only.
  - Time: Today and Last 30 Days.
  - Forecast: Tomorrow and Next 30 Days.
- The chart has 30 daily bars with rounded tops in the accent colour. Horizontal grid lines carry value labels on the right, which replace "Up to N". A dim date is written under each week.
- The selected day (today at first) gets a light rounded highlight, and selecting a day shows its values in the headline numbers. This replaces "Select a day to see its values".
- Failed is stacked on top of Passed in the same accent at about 38 % opacity, with a 2 px gap. **No hatching, and no average line.**
- Round previous and next buttons page back or forward by 30 days. Next is disabled on the current period. The forecast covers the next 30 days.
- Below the card, a "Memory" boxed list with three rows, each with its value at the end: Expected retention, Pass rate (last 30 days), Time studied (last 30 days). It replaces the three tiles.
- At 360 px the bars are about 7 px wide, so a day can also be selected by keyboard (arrow keys) and by touch on a wider hit area.
- Failed never depends on colour alone: the screen-reader text of the chart and of the selected day gives the failed count.
- Colours come only from named and accent colours, in light and dark mode. Paging, period sums and labels are pure functions with tests.

**5. #127: automatic backup folder** (Opus).
- One setting in a small Preferences dialog (`AdwPreferencesDialog`, opened from the main menu): "Automatic Backups", with a folder chosen through `GtkFileDialog::select_folder` (portal, so it works in the snap) and "Off" by default.
- When Rappel closes, if the collection changed since the last backup and none was made today, it writes a normal export (format v2, with media) named `Rappel backup YYYY-MM-DD.rappel` into the folder.
- It keeps the 14 newest files that match that exact pattern and deletes older ones (pre-approved rule above). It never touches other files.
- Failures (folder gone, disk full) never block closing and never lose data. The next start shows a toast saying the backup failed and why.
- Tests in core: name pattern, retention choice (pure), "changed since last backup", and once per day across time zones.
- A test checks that a symlink or a look-alike file name is never deleted.
- ADR.

**6. Test-week issues** (Sonnet, or Opus if the issue needs logic). Any issue labelled `bug` or `polish` filed before the batch starts and not covered above. If there are none, skip this slice. #48 (Orca) stays with the maintainer.

**7. The snap** (Opus).
- **First, the feasibility check, written as an ADR before any packaging:**
  - Which `gnome` extension and base (core26 expected) provides libadwaita ≥ 1.8, GTK 4 and GtkSourceView 5.
  - Whether glycin's bubblewrap sandbox runs under strict confinement.
  - Whether the GStreamer plugins on the ADR 0056 allow-list are present.
  - Where the data directory lands (`$SNAP_USER_DATA/.local/share/rappel`).
  - That `GtkFileDialog` folders chosen through the portal (export, import, backup folder) stay usable across restarts.
  - If either stop condition is hit, stop the slice and write the question.
- Install layout: the binary, `.mo` translations, desktop file, metainfo and icons are installed in their standard places. Use Meson only if the check shows it is needed; otherwise a small, tested install step in `snapcraft.yaml`.
- `snap/snapcraft.yaml`:
  - name `rappel`, strict confinement, the gnome extension;
  - plugs only from the standard desktop set: `desktop`, `desktop-legacy`, `wayland`, `x11`, `opengl`, `audio-playback`, `gsettings` if needed;
  - **no `home` or `removable-media`**, because files go through the portal;
  - licence GPL-3.0-or-later; title, summary, description and icon taken from the metainfo.
- A CI workflow (`.github/workflows/snap.yml`) builds the snap on `v*` tags and on manual dispatch, and attaches it as a workflow artifact. **It does not publish:** the maintainer uploads, and no store token goes into the repo or CI.
- The snap job must not slow the `check` job or run on docs-only PRs.

**8. Store listing** (Sonnet).
- Metainfo finished:
  - retake the screenshots with the new designs (deck page, All Cards, Statistics, review, phone size), still without the old key hints;
  - release notes for 0.4.0;
  - URLs pinned to the `v0.4.0` tag;
  - `appstreamcli validate --strict` passes.
- The snap's description matches it.

**9. Clean-up #5 and hardening round 5** (Opus).
- Remove dead code and duplication from slices 1 to 8, and bring ARCHITECTURE.md up to date.
- Attacks on the new paths:
  - a backup folder that is a symlink, read-only, full or removed while running;
  - 10,000 cards in the new rows, with heavy math;
  - paging stats to before the first review.
- Extend the ADR 0063 table. No migration, no format change.

**10. Version 0.4.0** (Sonnet).
- Bump to `0.4.0` in Cargo.toml, Cargo.lock, CHANGELOG and the metainfo release entry. Do **not** tag.
- Final report `docs/briefs/phase-3b-report.md`, containing:
  - decisions and questions;
  - screenshot paths;
  - which model ran each slice and any escalations;
  - a test checklist for the native build **and** for the snap installed from CI;
  - the exact data copy step from `~/.local/share/rappel` to `~/snap/rappel/current/.local/share/rappel`, since the snap cannot read the old folder.
- Draft PR `phase-3b` → `main`.

## After the batch (the maintainer, with the Lead engineer's exact commands)
1. Merge the draft PR, tag `v0.4.0`, create the GitHub release.
2. The snap workflow builds the `.snap`. Download it, `sudo snap install --dangerous ./rappel_0.4.0_amd64.snap`, copy the data folder, and run the snap checklist.
3. `snapcraft upload --release=beta rappel_0.4.0_amd64.snap` (decided 9 Nov: 0.4.x goes to **beta**; the stable channel waits for 1.0). The first upload may wait for a short store review.
