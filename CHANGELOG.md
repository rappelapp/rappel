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
- Dependency: `getrandom` in `cards-core` (random bits for ids; already in the tree through `fsrs`).

### Changed
- SPEC §3: ids are TEXT UUIDv7, `cards.seq` is the FTS5 rowid, and card deletion will be a soft delete.
- `Error::CreateDataDir` renamed to `Error::CreateDir` (also used for the backups directory).
