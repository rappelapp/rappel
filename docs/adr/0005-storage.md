# 5. SQLite storage, migrations and backups

- Status: Accepted
- Date: 2026-10-15

## Context

Cards needs a local database before any domain or UI code exists. SPEC §1 says losing review history is the worst possible bug, and SPEC §3 sets WAL, foreign keys, `integrity_check` at startup, backups with the online backup API, file mode `0600` and directory mode `0700`. The maintainer decided:
- ids are UUIDv7 stored as TEXT;
- `review_events` is the source of truth and card scheduling state is a cache that can be rebuilt;
- timestamps are UTC milliseconds.

## Decision

**SQLite library.** `rusqlite` with the `bundled` feature. SQLite (3.53 today) is compiled into the binary, so every machine and CI run the same version, FTS5 is available for search, and no system `libsqlite3-dev` is needed. The `backup` feature exposes the online backup API.

**Schema (migration 0001).**
- Tables: `decks`, `cards`, `review_events`, `card_schedule`.
- Every table is `STRICT`, so SQLite rejects values of the wrong type.
- Ids are lowercase 36-character UUIDv7 strings, enforced with a `CHECK`.
- Timestamps are `INTEGER` UTC milliseconds.
- `cards.seq INTEGER PRIMARY KEY` is an internal integer key. FTS5 external-content tables need a stable integer rowid, and SQLite's hidden rowid can change on `VACUUM` unless it is an `INTEGER PRIMARY KEY` column. `seq` is never exported or shown.
- `grade` is `'fail'` or `'pass'`.
- `review_events` is append-only. `BEFORE UPDATE` and `BEFORE DELETE` triggers abort with an error.
- Foreign keys use `ON DELETE RESTRICT` (deck → cards, card → reviews). A card with reviews therefore cannot be deleted, and the card-delete feature will be a soft delete.
- `card_schedule` is the cache. Deleting a card deletes its row (`CASCADE`), and a card with no row is new. Any later migration may drop and recreate this table, because the scheduler rebuilds it from `review_events`.
- `undoes`, `suspended`, tags, settings, media and FTS each arrive as their own migration with the slice that needs them.

**Migrations.**
- The schema version is `PRAGMA user_version`. `MIGRATIONS` is a static list of embedded SQL files, numbered from 1 with no gaps.
- Each pending migration runs in its own `BEGIN IMMEDIATE` transaction, which runs the SQL, sets `user_version` and runs `PRAGMA foreign_key_check`. Any error or violation rolls back that migration, and the database stays at the last good version.
- A database with a `user_version` above what the build knows, or a negative one, is refused untouched.
- A database with tables but `user_version = 0` is refused untouched (`NotACardsDatabase`). Migration 0001 sets the version in the same transaction that creates the tables, so such a file belongs to another application.
- If a migration's SQL ends the transaction itself, the version is not recorded and the open fails.
- Migrations run with foreign keys on. The first migration that rebuilds a table must also add a way to turn foreign keys off around its transaction; otherwise `DROP TABLE` cascades or is blocked (see the module docs).
- A test pins a 64-bit FNV-1a hash of every migration's SQL. Editing a merged migration fails the test, and getting past it means visibly editing the pin, which review catches.
- Another test builds a database at each version N-1 using the real code and migrates it to N, so every future migration is upgrade-tested automatically.

**Opening order.**
1. Create the directory (`0700`) and the file (`0600`). An existing file and its `-wal`/`-shm`/`-journal` files are tightened to `0600`. SQLite gives the side files it creates the same mode as the database file.
2. Set the connection-only pragmas (`foreign_keys=ON`, `trusted_schema=OFF`, `busy_timeout`).
3. Run `PRAGMA integrity_check`. Any problem, or a file that is not a database at all, returns `IntegrityCheckFailed` before anything is written.
4. Check the version (too new, negative, or not a Cards database → refuse). This happens before step 5 because switching to WAL rewrites header bytes, and a file we refuse must stay byte-for-byte unchanged.
5. Set `journal_mode=WAL` and `synchronous=FULL`. FULL is safer than NORMAL against power loss, and a flashcard app writes rarely.
6. Back up.
7. Migrate.

**Backups.**
- When:
  - before migrating an existing database;
  - otherwise once per UTC day, at the first open that day;
  - never for a brand-new empty database.
- Name: `backups/cards-YYYYMMDD-HHMMSS.sqlite3`, in UTC so the name never depends on the time zone or on daylight-saving changes. If a name is taken, `-2`, `-3`… is appended.
- Each backup is written to `<name>.partial` (pre-created `0600`) with the online backup API and switched to `journal_mode=DELETE` so it is one self-contained file. It then passes `PRAGMA quick_check`, is fsynced, and is renamed. The directory is fsynced afterwards. A file with a final name is always complete and checked.
- 14 backups are kept. The backup made during this open is always one of them, and the other 13 slots go to the newest names. Without that rule, backups dated by a clock that was once ahead could make the fresh backup sort oldest and get deleted at once, leaving a migration without a backup. Pruning only touches names it generates, and removes leftover `.partial` files.
- Failure handling:
  - If the pre-migration backup fails, the open fails and nothing is migrated.
  - If the daily backup or pruning fails, only a warning is logged. A healthy database stays usable, and the next open tries again.
- The copy runs in 256-page steps. If the source stays locked for about 5 s, it gives up with `SQLITE_BUSY` instead of retrying forever.
- UTC date fields come from Hinnant's days-to-civil algorithm (`time.rs`) rather than a date crate. It is about 20 lines and is tested against an independent inverse over ±800 000 days.

## Consequences

- If the integrity check fails, the app cannot open the database. Recovery means restoring a backup, which needs a UI; until then the user restores a file by hand. Refusing is safer than writing to a damaged file.
- The `UPDATE`/`DELETE` triggers also bind future migrations. A migration that has to rebuild `review_events` must drop and recreate them inside the migration.
- Bundled SQLite adds about 1–2 MB to the binary and a C compile to clean builds. CI already has `build-essential`.
- `tracing` is now a `cards-core` dependency, used to log migrations and backups. It is not a UI crate.
- `cards_core::Error` exposes `rusqlite::Error` in some variants. That is fine for a workspace-internal crate.
