# 20. Rename the app from Cards to Rappel

- Status: Accepted
- Date: 2026-10-16
- Amends: ADR 0004 (data directory names), ADR 0005 (database file name), ADR 0007 (backup names), ADR 0017 (export extension)
- Amended: 2026-10-17, `.cards` files are no longer imported: they are format version 1, which only test versions wrote (ADR 0017)

## Context

The project is renamed from Cards to Rappel (French for "recall" or "reminder"). The old name was a plain noun that is also what the app stores, which made code, docs and search results ambiguous. The rename touches names the user can see, names on disk and names in the code. Existing exports (`.cards`) must keep importing, and the maintainer will move their own data by hand.

## Decision

**Renamed.** Crates `cards-core` → `rappel-core` and `cards` → `rappel` (binary `rappel`). App ID `io.github.rsmirke.Rappel`, resource prefix `/io/github/rsmirke/Rappel`, `rappel.gresource`, GObject types `Rappel*`. Environment variable `RAPPEL_DATA_DIR`. Data directories `~/.local/share/rappel` and `rappel-dev`. Database `rappel.sqlite3` (and its `.restore.*` and `.replaced-*` names), backups `rappel-YYYYMMDD-HHMMSS.sqlite3`. `Error::NotACardsDatabase` → `NotARappelDatabase`. Window title, dialogs and error texts say Rappel. Exports are written as `.rappel`, suggested as `rappel-YYYY-MM-DD.rappel`.

**Not renamed.**
- The word "cards" as the thing a deck holds: the `cards` table, `Card`, `storage/cards.rs`, "No Cards Yet", migrations (they are never edited, ADR 0005).
- The archive's internal layout: `cards.json` and `media/`. Import looks at the content, never the extension. The only place extensions matter is the file chooser, which shows both (`IMPORT_EXTENSIONS`). Renaming `cards.json` would break old files for no benefit.
- *Amended 2026-10-17:* `.cards` files are format version 1, which Rappel no longer reads (ADR 0017, format version 2). They stay in the file chooser, so choosing one gives the clear refusal ("made by an early test version of Rappel and cannot be imported") instead of the file not being there.
- Git history, tags, past PR titles, past changelog entries.

**No automatic migration of the old data folder.** Rappel never reads `~/.local/share/cards`. Silently adopting or moving a folder written by another app name risks mixing two databases (the failure ADR 0007 exists to prevent). To move data: **Export…** in the last Cards build, then **Import…** in Rappel. The import restores decks, cards and the whole review history. *(No longer possible since format version 2, ADR 0017: nothing had been released, and there was no such data worth keeping.)* Backups made by Cards are not carried over.

**Debug guard.** A debug build refuses `cards` next to `rappel` as a release directory (`LEGACY_RELEASE_DIR_NAME`), so development cannot touch data that has not been moved yet. Release builds can still be pointed at it explicitly with `RAPPEL_DATA_DIR`.

## Consequences

- Breaking for existing installs: new app ID, data folder, environment variable and database name. Settings made through the old app ID are not carried over (there are none yet).
- A Rappel database opened by an old Cards build is not possible (different folder), so no downgrade path to worry about.
- The debug guard can be dropped once nobody has data in the old folder.
