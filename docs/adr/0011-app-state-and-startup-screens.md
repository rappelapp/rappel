# 11. AppState and startup screens

- Status: Accepted
- Date: 2026-10-15

## Context

The core can open, check, back up, migrate and restore the database (ADR 0005, 0007), but the app only created the data folder. The UI now needs the open database, and a database that cannot be opened must be explained instead of crashing the app. SPEC §2 says the UI talks to `AppState` only, never to SQL.

## Decision

**`AppState` is an `Rc<RefCell<Store>>` handle.** `crates/app/src/state.rs` defines `AppState(Rc<Inner>)`, with `Inner { data_dir, store: RefCell<Store> }`. It is `Clone`, and a clone is just one more pointer to the same store.
- `Rc`, not `Arc<Mutex<…>>`: GTK widgets live on the main thread only, so the store never needs to be shared across threads. rusqlite's `Connection` cannot be shared between threads anyway.
- `RefCell`, because writes need `&mut Store`. Each `AppState` method borrows the store, does its work and releases it before returning. No method returns a borrow or calls into GTK while it holds one. So a signal handler that runs during another handler never finds the store borrowed, and the run-time borrow check cannot fail.
- `AppState` methods return core domain types and `cards_core::Error`. `crates/app` does not depend on rusqlite.
- It only has the operations the UI uses so far. Later slices add their own.
- The recovery page runs while no database is open, so there is no `AppState` instance yet. It uses two associated functions, `AppState::list_backups(dir)` and `AppState::restore_unopened(dir, backup)`. The UI still reaches storage only through `AppState`.

**Widgets get it by constructor injection.** The window keeps the handle in a `OnceCell<AppState>`. It is set once, when the database is open: at startup, or after a restore from the recovery page. Pages and dialogs will receive a clone from the window. A global or thread-local was rejected because it hides which widgets use the database and makes "no database yet" (a startup failure) implicit. A GObject subclass was rejected for now because there are no properties or signals to bind. It can come later if pages need change notifications.

**The database is opened in the first `activate`, not in `main()`.** GApplication is single-instance: a second launch only forwards `activate` to the running process and exits. Opening in `main()` would make that second process open the database too, back it up and possibly migrate it while the first has it open. Restore also relies on a single instance (ADR 0007). The open is synchronous: an integrity check and a daily backup of a flashcard database take milliseconds. Restore runs on a worker thread (`gio::spawn_blocking`) because it copies and checks a whole file. `Store` is `Send`, and a core test pins that.

**Startup errors map to full-window pages.** `crates/app/src/startup.rs` maps the error to a page with a pure, unit-tested function:

| Error | Page |
|---|---|
| `IntegrityCheckFailed`, `NotACardsDatabase`, `OrphanedSidecar`, `InterruptedRestore` | Recovery: explanation, backups newest first, Restore with an `AdwAlertDialog` confirmation |
| `UnsupportedVersion` | Too new: explanation and data folder, **no restore**. The file is fine; this build is too old. Restoring would set the user back to an older backup for no reason. |
| anything else, including data folder errors | Error: short message, full error text, data folder when known |

`OrphanedSidecar` and `InterruptedRestore` go to recovery because restoring again is how ADR 0007 resolves them. After a successful restore the window takes the new store and shows the main page. A refused backup (`RestoreRefused`) gives a toast and leaves the list there to pick another one. Any other restore error may come after the old database was moved aside. In that case the page switches to the "restore did not finish" explanation, which then matches the files on disk. The application is held during a restore, so closing the window does not cut it off.

**`Store::open` reports what it only logged.** A failed daily backup or prune used to be a log line only. `Store::open` now also records it as an `OpenWarning`, and `Store::take_open_warnings` returns it. The app shows each warning as a toast that stays until dismissed. When the backup failed, a prune failure is not reported to the user again, since they almost always share a cause (the backup directory). Both are still logged.

**Untrusted text.** Paths and error messages can contain `<` or `&`. `AdwStatusPage:description` is markup, so the app escapes them with `glib::markup_escape_text`. Toasts, rows and labels are set with markup off.

## Consequences

- Without a data folder the app shows the error page. It used to start with a toast, but there is nothing it could do without one.
- The single-instance assumption now covers opening too. A future command-line mode that runs without the GUI must not open a second `Store`.
- Startup blocks the main loop for the integrity check. If a large database ever makes it noticeable, the open can move to `spawn_blocking` behind a spinner page.
- With no backups (for example, the first day of use), the recovery page only says to leave the files alone. A better way out, such as starting a fresh database while keeping the old file, is left for later.
- The backup rows are built in code (one `AdwActionRow` per backup). The page and dialog layouts are in Blueprint.
