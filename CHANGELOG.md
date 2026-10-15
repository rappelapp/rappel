# Changelog

All notable changes to this project are documented here.
The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and the project aims to follow [Semantic Versioning](https://semver.org/).

## [Unreleased]

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
