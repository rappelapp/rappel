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

### Changed
- SPEC §3: ids are TEXT UUIDv7, `cards.seq` is the FTS5 rowid, and card deletion will be a soft delete.
- `Error::CreateDataDir` renamed to `Error::CreateDir` (also used for the backups directory).
