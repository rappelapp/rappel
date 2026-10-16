# Changelog

All notable changes to this project are documented here.
The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and the project aims to follow [Semantic Versioning](https://semver.org/).

## [Unreleased]

### Added
- The card editor is a Markdown editor (SPEC §4.4, ADR 0022): front and back use GtkSourceView with Markdown highlighting, a monospace font and line wrapping, in colours that follow the light or dark style. A live preview, rendered as on the review page, sits beside the text when the dialog is wide and below it when narrow; the Preview button in the header bar shows or hides it. The preview updates a quarter of a second after typing pauses, and shows only the first 32 KiB of each side (with a note), because laying out a very long paragraph froze the editor. **Ctrl+B** / **Ctrl+I** wrap the selection in `**bold**` / `*italic*` markers, or insert empty ones at the cursor; Ctrl+Z undoes them in one step. The dialog opens larger (960 × 640) and is still a bottom sheet on a narrow window.
- New dependency: `sourceview5` (Rust bindings for GtkSourceView 5). Building now needs the GtkSourceView 5 development files and introspection data (see README).
- Math is drawn (SPEC §4.2, ADR 0021, 0023). `$…$` sits in the text on the text baseline and wraps with it; `$$…$$` is a centred block. Both follow the light or dark theme live (drawn as vectors, meant to stay sharp on HiDPI screens), work on the review page and in the editor's preview, and carry their LaTeX source as the accessible label. HiDPI and screen readers are still to be checked in the running app. A formula wider than the card scrolls sideways on its own. Invalid LaTeX is shown as its source in the error style. Limits: 200 formulas per card side (the rest stay source), sources over 4 KiB and formulas over 128 em in either direction stay source. `\href` and `\url` show their text and are never links. Known limits: `\oiint` draws wrongly, `\mathbb{1}` is an upright 1.
- `fill_scratch_db --math`: a "Math" deck with the 15 formulas of the math spike, one per card, and a card with invalid LaTeX.
- New dependencies: `ratex-parser`, `ratex-layout`, `ratex-types` (MIT) and `ratex-cairo` (MIT, vendored in `third_party/ratex-cairo` with `cairo-rs` bumped to 0.22, used through `[patch.crates-io]`), which also bring `ratex-katex-fonts` (the KaTeX fonts, SIL OFL 1.1; credited in the README, licence text in `LICENSES/`). About 60 more packages in all, and up to 5 MB in the binary.

### Changed
- `scripts/check.sh` runs `cargo deny check -D license-exception-not-encountered`, so a changed licence file of the KaTeX fonts fails the gate (ADR 0023). `deny.toml`: `OFL-1.1` is allowed for `ratex-katex-fonts` only, and RUSTSEC-2026-0192 (`ttf-parser` unmaintained) is ignored with a reason.
- The quality gate needs a display, because a test starts GTK: CI runs it under Xvfb (`xvfb-run -a scripts/check.sh`, ADR 0024).
- **Review `RUSTSEC-2026-0192` in `deny.toml` by 2027-01-16**, earlier when RaTeX, `ab_glyph` or `fontdb` update, and remove it once RaTeX moves off `ttf-parser`. Every `ratex-*` bump: read `FONT_NOTICE.txt` of `ratex-katex-fonts` and recompute its hash in `deny.toml`, and check whether `third_party/ratex-cairo` can go.

### Documentation
- ADR 0021 (accepted): math rendering with RaTeX, from the `spike/math` experiment. SPEC §1 principle 5 and §4.2 amended. `\oiint` is a known limitation.
- ADR 0023: math widgets, the 128 em cap, measured costs and the licence gate.
- CLAUDE.md: don't launch the GUI beyond a quick start check.
- ADR 0024: inline formula scrollers have no scrollbar, and the tests need a display.
- CLAUDE.md: never discard uncommitted work. `.claude/settings.json` denies the usual discarding commands for Claude Code (`git restore`, `git checkout --` / `.` / `-f`, `git switch -f`, `git stash drop` / `clear`, `git reset --hard`, `git clean`, `--force`, `-f` and `+refspec` pushes, `git branch -D`, `rm -rf`). `--force-with-lease` stays allowed for rebased slice branches. It is a guard rail for the usual spellings, not a sandbox.

### Fixed
- An inline formula no longer leaves a gap after it. Pango reserved the width of its scroller's hidden scrollbar (about 46 px) whenever the formula was narrower than that, so `$x$` had some 35 px of blank space after it. Inline formulas now have a scroller without a scrollbar (ADR 0024); one wider than the card still scrolls with a touchpad swipe, touch, a tilt wheel or Shift+wheel (not a plain wheel), with the fade at its edge as the only hint.
- The card editor's front and back boxes have clean edges: a frame (1 px border, rounded corners that clip the text area) instead of a card whose shadow was cut by the text area's square corners. The background is still the view colour, darker in dark mode.

## 0.2.0-alpha.1 — 2026-10-16

### Added
- Edit a card during review: **E** or the pencil button in the header bar opens the card editor on the current card. After saving, the card is drawn again in place, and the answer, the timer and the session carry on (time in the editor counts towards the answer time, which is capped at 60 s as always); closing without saving changes nothing. The deck page's list shows the edit when you go back.
- Markdown cards (ADR 0019). Studying shows both sides rendered: paragraphs, headings, nested lists, code blocks, block quotes, rules, tables, emphasis, strong, strikethrough and inline code. Math (`$…$`, `$$…$$`) is shown as monospace source for now, and dollars used as prices (`$5-$10`, `\$`) stay text. Links are underlined and show their URL when hovered; they cannot be clicked. Images are a placeholder, and remote images are shown as text, never fetched. Raw HTML is shown as typed. A card whose back is empty shows only Fail and Pass after Show Answer. The card list shows each front without Markdown syntax.
- `cards-core`: the `content` module: `parse` (Markdown → `Document`, nesting capped at 16 levels), `plain_text` and `plain_summary`. New dependency: `pulldown-cmark`.
- `fill_scratch_db --markdown`: a deck with one card per Markdown construct. A count of 0 now adds no scratch deck.
- Export and import in the app (ADR 0018). The deck list has a main menu (☰, F10) with **Export…** and **Import…**. Export saves everything to `cards-YYYY-MM-DD.cards` and says where in a toast. Import checks the file first and shows what it would add, what it brings back that you deleted, and what is already there; with conflicts it lists them, using your own deck names and card fronts, and offers no import. After an import, the deck list and open deck pages update. The work runs in the background, so the window stays responsive, and closing the window lets it finish.
- `cards-core`: `Store::open_concurrent`, a second connection for a worker thread (never creates the database, refuses an unmigrated one).
- README: "Exporting and importing".
- `cards-core`: export the whole database to a `.cards` file (a zip archive with `cards.json`, format version 1, and an empty `media/`) with `Store::export_to`. Deleted rows are included, and the same database gives the same bytes. Import is two steps: `Store::plan_import` checks the untrusted file and summarises what is new, already present or in conflict; `Store::apply_import` backs up, then adds the new rows in one transaction and rebuilds the affected schedules. A deck or card deleted in the database but live in the file, and otherwise the same, is revived (a deck with the cards deleted with it); one deleted in the file but live in the database is left alone. Any other difference is a conflict, and any conflict refuses the whole import; ids must be `UUIDv7` (ADR 0017, #12). No UI yet.
- Recovery page with no backups: **Start Fresh**, after a confirmation, moves the database aside (never deleting it) and starts an empty one (ADR 0016, #16). Not offered, and refused by core, while backups exist (`Error::BackupsAvailable`) or an interrupted restore's copy waits.
- `cards-core`: `storage::start_fresh` and `Store::next_study_change`.
- Tests: the cache property test also adds, deletes and undeletes cards and decks; a file-backed store is reopened; a v2 backup is restored; migration 0005 moves the due date of a late Pass (#13).

### Changed
- **Breaking: the app is renamed from Cards to Rappel** (ADR 0020). Crates `rappel-core` and `rappel`, app ID `io.github.rsmirke.Rappel`, `RAPPEL_DATA_DIR`, data folder `~/.local/share/rappel` (`rappel-dev` for debug builds), database `rappel.sqlite3`, backups `rappel-YYYYMMDD-HHMMSS.sqlite3`. Exports are now `.rappel` files; the archive inside is unchanged (`cards.json`, format version 1), so existing `.cards` exports still import, and the import file chooser shows both. The old data folder is not migrated or read: to move your data, **Export…** in the last Cards build and **Import…** in Rappel. Debug builds also refuse the old `cards` data folder.
- Every card side looks the same: body text, normal weight, aligned to the start. A card that is one paragraph is no longer large and centred. Headings, code, quotes and tables keep their own styles.
- Tables have a border, lines between cells and a shaded header row, using theme colours only (`data/style.css`, the app's first stylesheet; ADR 0019).
- A deck or card deleted on both sides at different times counts as already present in an import, not as a conflict (ADR 0017).
- Deck names are limited to 1 KiB and card sides to 1 MiB (in bytes), the limits an import accepts, so every export can be imported again. The deck name dialog and the card editor explain a too long text under the field and disable saving; the store refuses it (`Error::DeckNameTooLong`, `Error::CardSideTooLong`).
- The deck list's "N new · N due" counts like the deck page: new cards still allowed today (20 a day), and due reviews plus step cards due within 20 minutes. `Store::list_decks` takes the study day's start.
- The deck page's subtitle and Study button update by themselves when a card becomes due or a new study day begins.
- Docs: CLAUDE.md and README say v0 is released as 0.1.0.

### Fixed
- An imported deck name with a NUL character crashed debug builds. Names and card sides with a NUL are now refused, by the import (a precise error naming the field) and when written (`Error::ContainsNul`).
- Closing the window with unsaved text in the card editor asks "Discard Changes?" first, as Escape does.
- The deck list cannot push a second page of the same deck when a row is activated again before the first push shows (#20).

## [0.1.0] — 2026-10-15

### Added
- Project skeleton: Cargo workspace with `cards-core` and `cards` crates, shared lints (`unsafe_code` forbidden), rustfmt, clippy and cargo-deny configuration.
- Minimal libadwaita window ("Cards", empty status page) built from a Blueprint template.
- Quality gate `scripts/check.sh`, pre-commit hook refusing commits on `main`, hook installer.
- Docs skeleton: README, ARCHITECTURE, this changelog, ADR 0001.
- `reviewer` Claude Code agent for independent slice review.
- ADR 0002: GTK/libadwaita minimum versions and the Blueprint build pipeline.
- CI: GitHub Actions workflow running `scripts/check.sh` on pull requests and pushes to `main` in an Ubuntu 26.04 container, with cargo caching (ADR 0003).
- Data directory: `CARDS_DATA_DIR`, else `$XDG_DATA_HOME` (default `~/.local/share`) + `cards` (release) or `cards-dev` (debug). Created with mode `0700`. Debug builds refuse the release directory. Path logged at startup; a toast appears if it cannot be created (ADR 0004).
- Dependencies: `thiserror` (core error type, as SPEC requires), `tempfile` (dev-only, for filesystem tests).
- Storage (ADR 0005):
  - `Store::open` with WAL, foreign keys, `synchronous=FULL` and an integrity check that refuses to open a damaged database.
  - A numbered migration framework that runs each migration in its own transaction. Old migrations are hash-pinned by a test.
  - Migration 0001: `decks`, `cards`, append-only `review_events` (enforced by triggers), and the `card_schedule` cache. Ids are UUIDv7 TEXT; timestamps are UTC ms.
  - Online-API backups before migrations and once per UTC day, keeping 14. The backup just made is never pruned. A failed daily backup or prune only logs a warning; a failed pre-migration backup stops the migration.
  - Damaged, too-new and non-Cards database files are refused before anything is written to them.
  - Files are created with mode `0600`, directories with `0700`.
- Dependencies: `rusqlite` with bundled SQLite (storage), `tracing` in `cards-core` (logging migrations and backups).
- Scheduling (ADR 0006):
  - `Scheduler::schedule` (one review) and `Scheduler::rebuild_state` (replay a card's review log) using FSRS-6 with default parameters and desired retention 0.90 (0.80–0.97 accepted). Fail = Again, Pass = Good.
  - New and failed cards get one 10-minute step. A Pass from the step gives an FSRS day interval.
  - Elapsed days are whole 24-hour periods since the last review, rounded. No time zone is involved.
  - Deterministic interval fuzz (Anki's ranges), seeded from card id and review count, so rebuilds are reproducible.
  - `schedule` refuses a review dated before the card's last review, so the log replays in the order it was recorded. Recording code must store `max(now, last_review_at)`.
  - Domain types `Grade`, `ReviewEvent`, `Phase`, `CardState`.
  - Migration 0002: `card_schedule` recreated with a `phase` column and rebuilt from `review_events` in the same transaction. `Store::review_events`, `Store::card_state`, `Store::rebuild_schedule_cache`.
- Dependencies: `fsrs` (the reference FSRS implementation, as SPEC requires), `proptest` (dev-only, property tests for replay).
- Backup restore in `cards-core` (ADR 0007), no UI yet:
  - `list_backups`: the backups, newest first, with time and size.
  - `restore_backup`: validates the backup (integrity, version, exact schema: object names and their columns) before changing anything. An older backup (e.g. schema v1) is migrated on that scratch copy first, so a failed migration refuses the restore with nothing changed. Moves the current database with its `-wal`/`-shm`/`-journal` aside to `cards.sqlite3.replaced-<timestamp>` instead of deleting it. Installs the backup atomically and opens it. Works when the current database is damaged.
  - `Store::open` refuses to start an empty database when the database file is missing but its `-wal`/`-journal`, or an interrupted restore's copy, is still there.
- README: "Recovering your data".
- Deck, card and review operations in `cards-core` (ADR 0008), no UI yet:
  - `UUIDv7` id generator whose ids strictly increase, within a millisecond, with a clock that goes backwards, and across restarts.
  - Decks: `create_deck`, `rename_deck`, `list_decks` with new / due-now counts, `delete_deck`. Names are trimmed and unique among live decks (`DuplicateDeckName`); a deleted deck's name can be reused.
  - Cards: `add_card`, `edit_card`, `card`, `list_cards`, `delete_card`.
  - Migration 0003: soft delete (`deleted_at` on decks and cards). Deleted items are hidden from every query and keep their review history. Deleting a deck soft-deletes its cards with the same `deleted_at`, so a later undo can restore exactly that batch.
  - `record_review`: the event (dated `max(now, last review)`) and the schedule cache update in one transaction.
  - `due_cards(deck, now)` and `new_cards(deck, limit)`.
  - Property test: after random reviews, clock jumps and restarts, the schedule cache equals a full rebuild from the log.
- Migrations can have a Rust backfill step, run in the migration's transaction (ADR 0010).
- Dependency: `unicode-normalization` in `cards-core` (NFC for deck name keys).
- Dependency: `getrandom` in `cards-core` (random bits for ids; already in the tree through `fsrs`).
- The app opens the database at startup through `AppState` (ADR 0011):
  - The main window shows the empty state "No Decks Yet".
  - A damaged database, a missing database file next to its journal, an interrupted restore, or another application's database shows a recovery page. It lists the backups newest first and restores the chosen one after an `AdwAlertDialog` confirmation, then the app continues.
  - A database from a newer Cards shows a page that explains this and offers no restore.
  - Any other startup failure shows a page with the message, the full error and the data folder.
  - A failed daily backup (or prune) is shown as a toast, not only logged: `Store::open` keeps it as an `OpenWarning` (`Store::take_open_warnings`).
- Deck list on the main page (ADR 0012):
  - Each deck shows its name and new/due counts. With no decks, the empty state has a "New Deck" button.
  - New Deck (header button, empty state, Ctrl+N) and Rename open a dialog. An empty name or a duplicate name (ignoring case and accent composition) disables the button, and a duplicate is explained under the entry.
  - Delete removes the deck at once and offers Undo in a toast.
  - Icon-only buttons have tooltips and accessible labels; the row menu's label names the deck.
- `Store::undelete_deck` in `cards-core`: restores a deleted deck and exactly the cards deleted with it; refuses, changing nothing, if a live deck took its name meanwhile.
- Deck page (ADR 0013):
  - Clicking a deck opens its page, titled with the deck name, with Back, swipe and Alt+← to return. The deck list's counts update on return.
  - The cards are listed by their front on one ellipsised line, in a list view that stays smooth with thousands of cards. With no cards, the empty state has an "Add Card" button.
  - Card editor with plain-text front and back: Ctrl+Enter saves, Ctrl+Shift+Enter saves and starts a new card in the same deck, a blank front disables saving, and closing with unsaved changes asks first.
  - Activating a card edits it; its row menu has Edit and Delete. Delete offers Undo in a toast, which also works after going back to the deck list.
  - Ctrl+N makes a new card on a deck page and a new deck on the deck list.
- `Store::undelete_card` in `cards-core`: restores a deleted card with its history; refuses, changing nothing, while its deck is deleted.
- `cards-core` example `fill_scratch_db`: fills a scratch data directory with 5,000 cards to try the card list.
- Studying (ADR 0014):
  - The deck page has a Study button, disabled when nothing is due and no new card is left for today, and says what there is to study ("3 due · 20 new").
  - The review page shows the front, then the back (Space or Enter), then Fail (1) or Pass (2 or Space). Card text is shown as plain text.
  - Due reviews come first in overdue order, with up to 20 new cards per deck per day spread among them. The study day starts at 4 a.m. local time.
  - Cards in the 10-minute step come back in the same session, and up to 20 minutes early once nothing else is left.
  - Each answer is saved at once, with the time from showing the card to the answer, capped at 60 s.
  - At the end, a summary (cards, pass rate, time) and Back to Deck.
- `cards-core`: `Store::study_input` and the pure `scheduling::session` (queue, interleaving, session, summary).
- `fill_scratch_db --due N`: makes N scratch cards due now, through the real scheduler, to try studying.

### Changed
- `Store::record_review` takes the time of the answer (`now`) from the caller, like the other time-dependent operations.
- Without a usable data folder the app shows an error page instead of starting with a toast.
- Deck names are unique ignoring case and Unicode composition ("Maths" = "maths", composed "É" = "E" + accent); accents still count. Migration 0004 adds and backfills `decks.name_key` (ADR 0010).
- Scheduling: a Pass on a review card is scheduled at least elapsed days + 1 out, before and after fuzz, as in Anki. Migration 0005 rebuilds the schedule cache for the new rule (ADR 0009).
- SPEC §3: ids are TEXT UUIDv7, `cards.seq` is the FTS5 rowid, and card deletion will be a soft delete.
- `Error::CreateDataDir` renamed to `Error::CreateDir` (also used for the backups directory).
- `Error::Random` holds a boxed error instead of `getrandom::Error`, so `getrandom` is no longer part of the `cards-core` API (ADR 0015, part of #13).
- The app's toasts, alert dialogs, navigation, row menus and list focus go through shared helpers in `crates/app/src/ui/`. No visible change.
- Lints are silenced only with `#[expect(.., reason = "..")]` (workspace lints `allow_attributes`, `allow_attributes_without_reason`).
- Docs: README, CLAUDE.md and SPEC say what v0 already does; CLAUDE.md has rules on focus, stacked PRs and UI that cannot be seen.

### Removed
- `Store::due_cards` and `Store::new_cards` in `cards-core`: unused since `Store::study_input`.

[Unreleased]: https://github.com/rsmirke/cards/compare/v0.1.0...HEAD
[0.1.0]: https://github.com/rsmirke/cards/releases/tag/v0.1.0
