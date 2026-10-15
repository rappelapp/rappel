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
- `src/error.rs`: the crate-wide `Error` enum (`thiserror`) and `Result` alias.
- `src/data_dir.rs`: `resolve_data_dir` (pure; takes a `DataDirEnv` and a `BuildProfile`) and `ensure_data_dir` (creates a directory with mode `0700`). Debug builds refuse the release directory. See ADR 0004.
- `src/time.rs`: `now_ms` (UTC milliseconds), `utc_parts` (calendar fields from milliseconds) and its inverse `utc_ms`.
- `src/domain/`: shared types: `Grade` (Fail/Pass), `ReviewEvent`, `Phase`, `CardState` (`New` or `Scheduled(Schedule)`).
- `src/scheduling/`: FSRS scheduling (ADR 0006), using the `fsrs` crate.
  - `mod.rs`: `Scheduler` (desired retention, default FSRS-6 parameters). `schedule(card_id, state, grade, now)` gives the state after one review; `rebuild_state(card_id, events)` replays a card's log by folding `schedule`. 10-minute step for new and failed cards; elapsed days are rounded 24-hour periods.
  - `fuzz.rs`: deterministic interval fuzz (Anki's ranges, seeded from card id + review count).
  - `tests.rs`: FSRS reference values and property tests (rebuild = review-by-review).
- `src/storage/`: SQLite storage (ADR 0005).
  - `mod.rs`: `Store::open(data_dir)` opens `<data_dir>/cards.sqlite3`. It checks integrity, sets WAL/FK, backs up and migrates.
  - `migrations/`: `MIGRATIONS` (numbered `NNNN_*.sql` files embedded with `include_str!`) and `migrate`. Merged migrations are hash-pinned by a test.
  - `backup.rs`: online-backup copies into `<data_dir>/backups/` (keeps 14), and the public `list_backups` (newest first, as `BackupInfo`).
  - `restore.rs`: `restore_backup` validates a copy of a backup (and migrates it there if it is older), moves the current database and its sidecar files aside to `cards.sqlite3.replaced-…`, installs the copy and opens it with `Store::open` (ADR 0007). `Store::open` refuses to start an empty database when the main file is missing but a `-wal`/`-journal` (`OrphanedSidecar`) or an interrupted restore's copy (`InterruptedRestore`) is there.
  - `schedule.rs`: `Store::review_events`, `Store::card_state` (reads the `card_schedule` cache) and `Store::rebuild_schedule_cache`. A migration with `rebuild_schedule: true` rebuilds the cache inside its own transaction.
  - `restore/tests.rs`: restore tests (v1 and v2 backups, interrupted restores, refusals that change nothing).
  - `tests.rs`: store-level tests.

  The schema is in `migrations/0001_initial.sql`; `0002_card_schedule_phase.sql` reshapes the `card_schedule` cache (adds `phase`).

### `cards` today

- `src/main.rs`: sets up `tracing` (level from `RUST_LOG`), registers the embedded gresource, resolves and creates the data directory (logs the path at `info`; on failure logs an error and the first window shows a toast), creates the `adw::Application` and shows the main window.
- `src/window.rs`: `CardsWindow`, an `adw::ApplicationWindow` subclass using a composite template; `add_toast` shows an `AdwToast`.
- `data/window.blp`: the window layout in Blueprint (`AdwToastOverlay` around header bar + empty `AdwStatusPage`).

## UI build pipeline

UI layout is written in Blueprint, never in Rust code or hand-written `.ui` XML.

```
data/*.blp ──blueprint-compiler──▶ $OUT_DIR/ui/*.ui ──glib-compile-resources──▶ $OUT_DIR/cards.gresource
                                                                                  │
                                         gio::resources_register_include! ◀───────┘ (embedded in the binary)
```

`crates/app/build.rs` drives both steps; `data/resources.gresource.xml` lists the files under the resource prefix `/io/github/rsmirke/Cards/`. Widgets load their template with `#[template(resource = "/io/github/rsmirke/Cards/<name>.ui")]`.

## Quality gate

`scripts/check.sh` runs `cargo fmt --all --check`, `cargo clippy --workspace --all-targets --locked -- -D warnings` (with `clippy::pedantic`, `unwrap_used`, `expect_used`), `cargo test --workspace --locked` and `cargo deny --locked check` (licences compatible with GPL-3.0-or-later, RustSec advisories, crates.io only). `scripts/pre-commit.sh` refuses commits on `main` and then runs the gate; `scripts/install-hooks.sh` installs it. `.github/workflows/ci.yml` runs the same `scripts/check.sh` on every pull request and on pushes to `main`, in an `ubuntu:26.04` container with the README's system packages and stable Rust (see ADR 0003).
