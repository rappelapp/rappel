# Rappel — SPEC (v0.2)

A native GNOME flashcard app. Rust, GTK4, libadwaita, Blueprint. Two grades only: **Fail** and **Pass**. Cards are Markdown with LaTeX math. Scheduling is FSRS. Everything is local: no network access, no accounts.

App name: **Rappel**. App ID: `io.github.rsmirke.Rappel`. Licence: **GPL-3.0-or-later**.

- Crates: `rappel-core` (`crates/core`, library) and `rappel` (`crates/app`, GTK binary).
- Data directory: `~/.local/share/rappel/`. Debug builds use `~/.local/share/rappel-dev/` so development never touches real data. Both can be overridden with the `RAPPEL_DATA_DIR` environment variable.

v0 is released as 0.1.0: decks, plain-text cards, review with Fail/Pass, FSRS, SQLite. v0.2 (**in progress**) builds on it. Done so far: export / import (§4.1), Markdown rendering on the review page, math rendered with RaTeX (§4.2, ADR 0021), editing the card on screen during review (part of §4.5), and the Markdown editor with live preview (§4.4; tags still to come).

---

## 1. Principles

1. **Data first.** Losing review history is the worst possible bug. Backups, export and migrations come before features.
2. **Core is UI-agnostic.** `rappel-core` holds all logic and has no GTK dependency. It must stay testable with plain `cargo test`.
3. **GNOME-native.** Follow the GNOME HIG. Use libadwaita widgets as designed. No custom colors, no web views.
4. **Few options, good defaults.** A setting needs a reason to exist.
5. **Untrusted content.** Card text may later come from imports. It is never interpreted as HTML or markup without escaping. Card content never causes network access or access to user files. Reading installed system fonts is allowed (ADR 0021).
6. **Adaptive from phone (360 px) to desktop, touch-friendly, ready for mobile Linux.**

---

## 2. Architecture

```
rappel/
├── Cargo.toml                 # workspace, [workspace.lints], shared deps
├── README.md  SPEC.md  CLAUDE.md  ARCHITECTURE.md  CHANGELOG.md
├── docs/adr/                  # Architecture Decision Records (NNNN-title.md)
├── deny.toml  rustfmt.toml  clippy.toml
├── LICENSE                    # GPL-3.0-or-later
├── scripts/check.sh           # the single quality gate (see CLAUDE.md)
├── scripts/pre-commit.sh  scripts/install-hooks.sh
├── .github/workflows/ci.yml   # runs scripts/check.sh (ADR 0003)
├── .claude/agents/reviewer.md
└── crates/
    ├── core/                  # rappel-core — no gtk/glib/adw
    │   └── src/
    │       ├── lib.rs
    │       ├── error.rs        # thiserror Error enum, Result alias
    │       ├── domain/         # Deck, Card, ReviewEvent, Grade, CardState, Tag, Settings
    │       ├── storage/        # Store, migrations/, backup.rs, queries
    │       ├── scheduling/     # FSRS wrapper, session queue, rebuild_state
    │       ├── content/        # Markdown -> Document (UI-agnostic block/inline tree)
    │       ├── media/          # content-addressed media store
    │       ├── stats/          # pure functions over review events
    │       └── interchange/    # JSON export / import (versioned format)
    └── app/                   # rappel — GTK binary
        ├── build.rs            # Blueprint (.blp) -> .ui -> embedded gresource
        ├── data/               # .blp files, resources.gresource.xml
        └── src/
            ├── main.rs  application.rs  config.rs
            ├── state.rs        # AppState: owns Store, exposes operations to UI
            ├── render/         # Document -> GTK widgets (labels, code, math)
            ├── window.rs       # main window (template in data/window.blp)
            ├── widgets/        # reusable widgets, each .rs + .blp
            ├── pages/          # deck, review, stats, browser
            └── dialogs/        # card editor, preferences, about, shortcuts
```

- UI talks to `AppState` only, never to SQL.
- Markdown parsing lives in core (`content`) and produces a `Document` tree. The GTK layer only turns that tree into widgets. This keeps parsing testable and reusable for a future WASM or mobile front end.
- Errors: `rappel-core` uses `thiserror`; the app turns core errors into user-facing `AdwToast`s or a startup problem page (no `anyhow`).
- Logging: `tracing` + `tracing-subscriber`, level from `RUST_LOG`. No card content in logs above `debug`.

---

## 3. Data model changes

Keep everything from v0 (migration 0001, ADR 0005). Ids are UUIDv7 stored as `TEXT`. Timestamps are UTC milliseconds. `cards` also has an internal `seq INTEGER PRIMARY KEY`, a stable integer key for FTS5 that is never exported. `review_events` is the source of truth; `card_schedule` is a cache rebuilt from it. Triggers forbid `UPDATE`/`DELETE` on `review_events`, and a card with reviews cannot be deleted (`ON DELETE RESTRICT`), so card deletion is a soft delete. Add, via numbered migrations:

```sql
ALTER TABLE cards ADD COLUMN suspended INTEGER NOT NULL DEFAULT 0;

tags(id TEXT PRIMARY KEY, name TEXT NOT NULL UNIQUE);
card_tags(card_id TEXT REFERENCES cards(id) ON DELETE CASCADE,
          tag_id  TEXT REFERENCES tags(id)  ON DELETE CASCADE,
          PRIMARY KEY(card_id, tag_id));

-- Undo without breaking the append-only log:
ALTER TABLE review_events ADD COLUMN undoes TEXT REFERENCES review_events(id);
-- An event with `undoes` set cancels the referenced event. rebuild_state skips both.

settings(key TEXT PRIMARY KEY, value TEXT NOT NULL);
-- desired_retention (0.80–0.97, default 0.90), max_new_per_day (default 20),
-- max_reviews_per_day (default 200)

media(hash TEXT PRIMARY KEY, mime TEXT NOT NULL, bytes INTEGER NOT NULL, created_at INTEGER NOT NULL);

-- Full-text search
CREATE VIRTUAL TABLE cards_fts USING fts5(front_md, back_md, content='cards', content_rowid='seq');
```

Storage rules:
- `PRAGMA journal_mode=WAL`, `foreign_keys=ON`, `PRAGMA integrity_check` at startup (log and toast if it fails).
- **Backups:** before any migration, and once per day at startup, copy the DB with the SQLite online backup API to `<data dir>/backups/rappel-YYYYMMDD-HHMMSS.sqlite3`. Keep a backup if it is from the last 24 hours, or the newest of its UTC day for each of the last 14 days, or the newest one overall, or the one just made or just restored (ADR 0026).
- DB and backup files are created with mode `0600`, directories `0700`.

---

## 4. Features for v0.2 (in priority order)

### 4.1 Export / import
- Export everything to one `.rappel` file (older `.cards` files import too, ADR 0020): a zip archive holding exactly `cards.json` and a `media/` folder (ADR 0017). `cards.json` has `"format_version": 1`, `exported_at`, and all decks, cards and the full review log, deleted rows included with their `deleted_at`. It also has `settings` (empty until settings exist) and `media` (empty until images exist; image files will go into `media/`). Tags join the format, as version 2, with the tags slice. A reader accepts every version up to its own.
- Deterministic: the same database exported at the same time gives the same bytes (lists sorted by id, fixed zip metadata). Only `exported_at` differs between exports.
- Import into an empty or existing DB in two steps: `plan_import` checks the file and returns a summary without writing anything (new / already present / conflicts, e.g. "12 decks, 840 cards, 5,210 reviews"); `apply_import` backs up the database, then writes in one transaction and rebuilds the schedule of the affected cards from the log.
- Merge rules: an id not in the DB is added; the same id with the same content is skipped. A row that differs only by being deleted is merged: deleted in the DB and live in the file, it is revived (`deleted_at` cleared; a revived deck also revives the cards deleted with it, as `undelete_deck` does, except those the file has, which follow their own row); live in the DB and deleted in the file, it is skipped (an import never deletes); deleted on both sides, at whatever times, it is already present. The same id with any other difference, or a live deck name (revived ones included) already used by another live deck, is a conflict, and any conflict refuses the whole import. The summary also counts revived rows. Importing the same file twice changes nothing.
- The file is untrusted. A file is refused if any of these fail: size limits; the exact entry names (nothing is extracted, so no zip-slip); a supported format version; every id a lowercase `UUIDv7`; references that resolve; valid grades and timestamps; no NUL character in a deck name or card side.
- Round-trip test: export → import into empty DB → export again produces an identical file.

### 4.2 Math
- `$...$` inline and `$$...$$` display math in Markdown.
- Rendering: evaluate **RaTeX** first (pure-Rust, KaTeX-compatible, has `ratex-cairo` and a `ratex-gtk4` widget). Draw inline math as vectors in the text flow, on the text baseline; display math as a centered block (ADR 0021). Formula color follows the theme foreground (works in dark mode).
- Fallback if RaTeX doesn't work out: show the LaTeX source in monospace. Never a web view.
- Invalid LaTeX shows the source with an error style, never crashes.
- Record the decision in `docs/adr/`.

### 4.3 Images
- Paste or drag an image into the card editor. It is copied into the media store (`<data dir>/media/<sha256>.<ext>`) and referenced as `![](media:<sha256>.<ext>)`.
- Only `media:` URLs render. Remote URLs and arbitrary file paths are shown as text, never fetched.
- Accept PNG, JPEG, WebP, SVG? → **no SVG** for now (script/external-reference risk). Max 10 MB.

### 4.4 Editor
- `GtkSourceView` 5 with Markdown highlighting, monospace, line wrapping.
- Live preview below or beside the editor (toggle), using the same renderer as review.
- Tags field (comma-separated or `AdwEntryRow` chips). *Comes with the tags slice (migration, export), not with the editor slice.*
- Ctrl+Enter saves, Ctrl+Shift+Enter saves and starts a new card in the same deck.

### 4.5 Review
- Undo last grade (Ctrl+Z) via an `undoes` event.
- Daily limits from settings. New cards interleaved with reviews.
- Card actions during review: Edit (E), Suspend, Delete (with confirmation).
- Session summary at the end: cards reviewed, pass rate, time spent.

### 4.6 Browser and search
- A "All Cards" page: search field (FTS5), filter by deck, tag, state (new / due / suspended).
- Multi-select: move to deck, add tag, suspend, delete.

### 4.7 Stats
- Per deck and global: reviews per day (last 30 days, bar chart), pass rate, due forecast (next 14 days), and **expected retention** today = mean FSRS retrievability over reviewed cards.
- Drawn with `GtkDrawingArea` + Cairo, theme colors only. No chart library.
- All computations are pure functions in `core::stats`, tested.

### 4.8 Polish
- `AdwPreferencesDialog`: desired retention, daily limits, backup folder "Open" button.
- `AdwAboutDialog`, keyboard shortcuts dialog, primary menu.
- Remember window size and the last selected deck.
- Every icon-only button has a tooltip and an accessible label. Full keyboard navigation.
- Empty states (`AdwStatusPage`) everywhere a list can be empty.
- Nested decks: **not** in v0.2.

---

## 5. Later (not v0.2)
- Meson build + Flatpak manifest + AppStream metainfo + app icon + `.desktop` file (needed to install it properly and to look like a GNOME core app).
- Translations (gettext), French first.
- Anki `.apkg` import.
- FSRS parameter optimisation from the user's own review log.
- Sync.
- Cloze cards.
- Obsidian-style in-place live preview in the editor (formulas rendered inside the text being edited, source shown where the cursor is). Needs a spike first, with its own ADR (next free number).

## 6. Never in this project
- More than two grades. Card links, proximal-zone suggestions, prerequisite graphs: those belong to the separate method app.
- Network access, telemetry, accounts.
