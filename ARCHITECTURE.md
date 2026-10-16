# Architecture

This file describes the code **as it is now**. The target layout is in [SPEC.md §2](SPEC.md#2-architecture); update this file whenever a slice changes the structure.

## Workspace

A Cargo workspace (`Cargo.toml`) with two crates. Shared dependency versions, package metadata and lints live in the root `Cargo.toml` (`[workspace.dependencies]`, `[workspace.package]`, `[workspace.lints]`); each crate opts in with `.workspace = true` / `[lints] workspace = true`.

| Crate | Path | Kind | Role |
|---|---|---|---|
| `cards-core` | `crates/core` | library | All logic. No UI dependency, testable with plain `cargo test`. |
| `cards` | `crates/app` | binary | GTK4 + libadwaita front end. Talks to core only. |

Dependency direction: `cards` → `cards-core`, never the reverse. `cards-core` must not depend on gtk4, glib, gio, libadwaita or anything UI.

### `cards-core` today

- `src/lib.rs`: the app name and ID constants and `is_valid_app_id`; re-exports `Error` and `Result`.
- `src/error.rs`: the crate-wide `Error` enum (`thiserror`) and `Result` alias. `Random` boxes its cause so `getrandom` stays out of the API; `rusqlite` and `fsrs` errors stay visible (ADR 0015).
- `src/data_dir.rs`: `resolve_data_dir` (pure; takes a `DataDirEnv` and a `BuildProfile`) and `ensure_data_dir` (creates a directory with mode `0700`). Debug builds refuse the release directory. See ADR 0004.
- `src/id.rs`: `IdGen`, strictly increasing `UUIDv7` strings (a 122-bit counter with random low bits; `observe` raises it above an id read from the database). See ADR 0008. `is_uuid_v7` checks ids from outside (an import, ADR 0017).
- `src/time.rs`: `now_ms` (UTC milliseconds), `utc_parts` (calendar fields from milliseconds) and its inverse `utc_ms`.
- `src/domain/`: shared types and `deck_name_key` (the uniqueness key of deck names): `Deck`, `DeckSummary` (deck + new/due counts), `Card`, `Grade` (Fail/Pass), `ReviewEvent`, `Phase`, `CardState` (`New` or `Scheduled(Schedule)`).
- `src/scheduling/`: FSRS scheduling (ADR 0006), using the `fsrs` crate.
  - `mod.rs`: `Scheduler` (desired retention, default FSRS-6 parameters). `schedule(card_id, state, grade, now)` gives the state after one review; `rebuild_state(card_id, events)` replays a card's log by folding `schedule`. 10-minute step for new and failed cards; elapsed days are rounded 24-hour periods. A Pass on a review card is at least elapsed + 1 days, before and after fuzz (ADR 0009).
  - `fuzz.rs`: deterministic interval fuzz (Anki's ranges, seeded from card id + review count).
  - `session.rs`: the study session, pure (no SQL, no clock): `StudyInput`, `interleave` (new cards spread evenly among reviews), `Session` (`next(now)`: due step cards, then the queue, then step cards up to 20 minutes early; `graded` re-queues cards left in the 10-minute step), `Summary`, `capped` (60 s), `NEW_PER_DAY` (20). Tests in `session/tests.rs`. See ADR 0014.
  - `tests.rs`: FSRS reference values and property tests (rebuild = review-by-review).
- `src/interchange/`: export and import of the whole database as a `.cards` file (ADR 0017). No UI.
  - `mod.rs`: the public types (`ExportSummary`, `ImportPlan` and its `ImportSummary` with `TableCounts` and `Conflict`s, `ImportReport`, `ImportProblem` / `FieldProblem` for a refused file) and the internal `Limits` (file, JSON, text sizes).
  - `format.rs`: the serde types of `cards.json` (`FORMAT_VERSION` 1), fields in file order, unknown fields denied.
  - `archive.rs`: writes the zip deterministically (`cards.json` then `media/`, fixed timestamp and permissions) and reads `cards.json` back after checking the entries are exactly those two (names compared as bytes, the count checked against the end record, a size cap on decompression).
  - `validate.rs`: the checks that need no database: version first, then ids (`id::is_uuid_v7`), uniqueness, references, grades, timestamps, names, text sizes, live cards in deleted decks.
  - `export.rs`: `Store::export_to` (atomic write, mode `0600`) and `export_bytes_at`, reading every table in one read transaction, sorted by id.
  - `import.rs`: `Store::plan_import` (read and compare, no writes) and `Store::apply_import` (no-op if nothing is new; else backup, then one transaction that compares again, inserts and rebuilds the schedule of cards with new events via `storage::rebuild_cards`).
  - `tests.rs`: a fixed fixture that pins the format, round trips, merges, each conflict, tampered files, and a property test over random databases.
- `src/storage/`: SQLite storage (ADR 0005).
  - `mod.rs`: `Store::open(data_dir)` opens `<data_dir>/cards.sqlite3` in named steps: `connect`, the integrity and version checks, `apply_write_settings` (WAL, `synchronous=FULL`, foreign keys), `back_up`, then the migrations. A failed daily backup or prune does not stop it. It is kept as an `OpenWarning` for `take_open_warnings`. `Store` owns the connection, an `IdGen`, a `Scheduler` and its backup directory (`None` for in-memory test stores; `interchange` uses it to back up before an import). `observe_max_id` raises the generator above a table's largest id before each write that issues one.
  - `decks.rs`: `create_deck`, `rename_deck` (trimmed, unique among live decks), `list_decks(now, day_start)` (each deck's new and due counts are the lengths of its `study_input`, so the deck list and the deck page agree), `delete_deck` (soft; the deck and its live cards share one `deleted_at`), `undelete_deck` (brings back exactly that batch, ADR 0012).
  - `cards.rs`: `add_card`, `edit_card` (returns the stored card), `card`, `list_cards`, `delete_card` (soft), `undelete_card` (refused while the card's deck is deleted, ADR 0013).
  - `review.rs`: `record_review(.., now)` (event + cache in one transaction, time `max(now, last review)`), `study_input(deck, now, day_start)` (due reviews, step cards due within 20 minutes, new cards first reviewed since `day_start`, and today's remaining new cards; `study_input_in` is the same on a connection inside a read transaction, shared with `list_decks`), `next_study_change(deck, now)` (the next time `study_input` would offer one more card: a review's due time, or a step card's due time minus 20 minutes).
  - Every public write has a private `*_at(now)` twin for tests, except `record_review`, which takes `now` itself (ADR 0014); `decks/`, `cards/`, `review/` hold their `tests.rs` (the review one has the cache-equals-rebuild property test). Deleted rows are hidden from every deck, card and queue operation; a deleted card's history and cache stay readable by card id (`review_events`, `card_state`) (ADR 0008).
  - `migrations/`: `MIGRATIONS` (numbered `NNNN_*.sql` files embedded with `include_str!`) and `migrate`. A migration may also have a Rust `backfill` step (run after its SQL, in its transaction) and ask for a schedule cache rebuild. Merged migrations are hash-pinned by a test.
  - `backup.rs`: online-backup copies into `<data_dir>/backups/` (keeps 14), and the public `list_backups` (newest first, as `BackupInfo`).
  - `restore.rs`: `restore_backup` validates a copy of a backup (and migrates it there if it is older), moves the current database and its sidecar files aside to `cards.sqlite3.replaced-…`, installs the copy and opens it with `Store::open` (ADR 0007). `start_fresh` moves them aside the same way and opens a new, empty database; it refuses while an interrupted restore's copy waits (ADR 0016). `Store::open` refuses to start an empty database when the main file is missing but a `-wal`/`-journal` (`OrphanedSidecar`) or an interrupted restore's copy (`InterruptedRestore`) is there.
  - `schedule.rs`: `Store::review_events`, `Store::card_state` (reads the `card_schedule` cache) and `Store::rebuild_schedule_cache`; `rebuild_cards` (some cards only, in the caller's transaction, for an import); `upsert_schedule` / `write_card_state`, the one way cache rows are written. A migration with `rebuild_schedule: true` rebuilds the cache inside its own transaction.
  - `restore/tests.rs`: restore tests (v1 and v2 backups, interrupted restores, refusals that change nothing).
  - `tests.rs`: store-level tests.

  The schema is in `migrations/0001_initial.sql`; `0002_card_schedule_phase.sql` reshapes the `card_schedule` cache (adds `phase`); `0003_soft_delete.sql` adds `deleted_at` to `decks` and `cards` and the live-unique deck name index. `0004_deck_name_key.sql` moves that uniqueness onto `decks.name_key` (NFC + lowercase, filled in by the migration's Rust backfill step, ADR 0010). `0005_rebuild_for_interval_floor.sql` only rebuilds the schedule cache after the scheduler change of ADR 0009.
- `examples/fill_scratch_db.rs`: fills a scratch data directory with one deck of many cards (default 5,000) through the public `Store` API, to check the card list's speed. With `--due N`, the first N cards get a past review history (recorded with `record_review` at past times) so they are due now. It refuses the app's own release and debug data directories and any path with `..`.

### `cards` today

- `src/main.rs`: sets up `tracing` (level from `RUST_LOG`), registers the embedded gresource and creates the `adw::Application`. The first `activate` runs `startup::start` and shows the main page or the page for the problem. Later activations only present the window. The database is opened there and not in `main()`, so a second launch, which only forwards `activate`, never opens it (ADR 0011).
- `src/state.rs`: `AppState`, a cheap-to-clone `Rc<RefCell<Store>>` handle and the UI's only way to the database. No method holds a borrow past its return. The associated functions `list_backups` and `restore_unopened` serve the recovery page, which runs before any database is open.
- `src/clock.rs`: `day_start_ms` (04:00 local of the current study day, computed with GLib's time zones, DST-tested), `next_day_start_ms` (04:00 local of the next calendar day, 23 or 25 hours away across a DST change), and their `local_*` forms. Core gets the result as a timestamp (ADR 0014).
- `src/startup.rs`: resolves and creates the data directory, opens `AppState`, and maps a failure to a `Problem` (recovery, too new, other), unit-tested and without GTK. `data_folder_line` is the "Data folder: …" line the problem pages show.
- `src/window.rs`: `CardsWindow`, an `adw::ApplicationWindow` subclass using a composite template. It holds the `AppState` (`OnceCell`, set once). `show_main` turns open warnings into toasts and shows the main page; `show_problem` shows a startup problem page. Its `close_request` looks through the open dialogs for a `CardEditor` with unsaved text and asks its "Discard Changes?" first, as Escape does.
- `src/ui/`: small helpers shared by pages, dialogs and rows; no layouts. Use them instead of writing a local copy.
  - `toast.rs`: `persistent_toast` (plain text, stays until dismissed), `show_toast_from` (the overlay around a widget), `log_failure` / `show_error` (a generic warning in the log, the message with user content only at debug level, SPEC §2), `undo_toast` (High priority, "_Undo"), and `warning_message` (the toast text for an `OpenWarning`).
  - `dialog.rs`: `builder` / `object` / `alert` load objects from the bundled `.ui` files (the resource prefix is in one place) and log a missing object.
  - `nav.rs`: `navigation_view` (the ancestor `AdwNavigationView`, logged if missing) and `pop`. Its module comment records the focus rule: focus a pushed page in its `shown` handler.
  - `rows.rs`: `install_id_action` (a page action taking an id string), `forward_id` (a row's `row.*` action forwarding to it), `label_menu_button` ("Actions for …").
  - `focus.rs`: `index_after_removal`, the row to focus after a removal (or `None`: the empty state's button). Pure and unit-tested.
  - `callback.rs`: `Callback`, a shared closure that template structs can derive `Debug` with.
  - `mod.rs`: `init_once`, which sets a `OnceCell` of a widget under construction and logs instead of ignoring a second set.
- `src/pages/recovery_page.rs`: `RecoveryPage` explains why a backup is needed, lists backups newest first, confirms with an `AdwAlertDialog` (`data/restore_dialog.blp`) and restores on a worker thread. With no backups (and no interrupted restore's copy waiting) it offers Start Fresh instead (`data/start_fresh_dialog.blp`, ADR 0016). Its callback says how the database was obtained (`Recovered::Backup` / `Fresh`), for the window's toast.
- `src/pages/decks_page.rs`: `DecksPage`, the root `AdwNavigationPage`. It shows the deck list (or the "No Decks Yet" empty state) and handles create, rename, and delete with an Undo toast, through `AppState`. Its class actions `decks.rename` / `decks.delete` take a deck id. Activating a deck pushes its `DeckPage`, unless a deck page is already on top (key repeat, #20); coming back reloads the counts. See ADR 0012.
- `src/pages/deck_page.rs`: `DeckPage`, one deck's `AdwNavigationPage`, built per open. Its cards are a `GtkListView` over a `gio::ListStore` of `CardObject` in creation order, updated item by item (add, edit, delete, Undo). Class actions `deck.new-card`, `deck.study`, `deck.edit-card` / `deck.delete-card` (card id). Its subtitle says what there is to study, and Study is disabled when that is nothing. A one-shot timer re-checks this when the next card becomes due or the next study day begins (capped at one hour per wait). See ADR 0013, 0014.
- `src/pages/review_page.rs`: `ReviewPage`, a pushed study session: front, Show Answer, then Fail / Pass, and a summary at the end. Keys in the bubble phase (Space/Enter reveal; 1 Fail; 2 or Space Pass). Each answer goes through `record_review` at once. Pure text helpers `summary_text`, `duration_text` are unit-tested. See ADR 0014.
- `src/widgets/` holds the row widgets (each with a `data/*.blp` layout) and `CardObject` (a list item, no layout).
- `src/widgets/card_object.rs`: `CardObject`, a list item holding a `Card` and its one-line preview (`front_preview`, unit-tested).
- `src/widgets/card_row.rs`: `CardRow`, the recycled row widget (preview label and a menu); `bind`/`unbind` point it at a card, and its `row.*` actions forward to the page.
- `src/widgets/deck_row.rs`: `DeckRow`, an `AdwActionRow` with the name, the new/due counts and a menu. Its `row.*` actions forward to the page with the row's deck id.
- `src/dialogs/card_editor.rs`: `CardEditor`, an `AdwDialog` with plain-text front and back. Ctrl+Enter saves, Ctrl+Shift+Enter saves and starts a new card; a blank front disables saving; closing with changes asks first (`data/discard_dialog.blp`). Reports each save to the deck page as `Saved`.
- `src/dialogs/deck_name_dialog.rs`: the create/rename `AdwAlertDialog` (`ask`). Its pure name check `check_deck_name` (empty / duplicate by `deck_name_key` / valid) gives the inline error.
- `data/window.blp`: `AdwToastOverlay` around a `GtkStack` with the pages `main` (an `AdwNavigationView` holding `DecksPage`), `recovery`, `too-new` and `error`, each with its own header bar. The window action `win.new-deck` opens the name dialog; `win.new` (Ctrl+N) opens the card editor on a deck page and the name dialog on the deck list, and does nothing on the review page. Both are enabled only on the main page.
- `data/decks_page.blp`, `data/deck_row.blp`, `data/deck_name_dialog.blp`, `data/deck_page.blp`, `data/card_row.blp`, `data/card_editor.blp`, `data/discard_dialog.blp`, `data/review_page.blp`: the layouts of the above.
- `data/recovery_page.blp`, `data/restore_dialog.blp`, `data/start_fresh_dialog.blp`: the recovery page and its confirmations.

## UI build pipeline

UI layout is written in Blueprint, never in Rust code or hand-written `.ui` XML.

```
data/*.blp ──blueprint-compiler──▶ $OUT_DIR/ui/*.ui ──glib-compile-resources──▶ $OUT_DIR/cards.gresource
                                                                                  │
                                         gio::resources_register_include! ◀───────┘ (embedded in the binary)
```

`crates/app/build.rs` drives both steps; `data/resources.gresource.xml` lists the files under the resource prefix `/io/github/rsmirke/Cards/`. Widgets load their template with `#[template(resource = "/io/github/rsmirke/Cards/<name>.ui")]`.

## Quality gate

`scripts/check.sh` runs `cargo fmt --all --check`, `cargo clippy --workspace --all-targets --locked -- -D warnings` (with `clippy::pedantic`, `unwrap_used`, `expect_used`, and `allow_attributes` / `allow_attributes_without_reason`: a lint is silenced only by `#[expect(.., reason = "..")]`, which fails the build once it is no longer needed), `cargo test --workspace --locked` and `cargo deny --locked check` (licences compatible with GPL-3.0-or-later, RustSec advisories, crates.io only). `scripts/pre-commit.sh` refuses commits on `main` and then runs the gate; `scripts/install-hooks.sh` installs it. `.github/workflows/ci.yml` runs the same `scripts/check.sh` on every pull request and on pushes to `main`, in an `ubuntu:26.04` container with the README's system packages and stable Rust (see ADR 0003).
