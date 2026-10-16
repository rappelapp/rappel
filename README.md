# Cards

A native GNOME flashcard app. Two grades only (Fail and Pass), Markdown cards with LaTeX math, FSRS scheduling. Everything stays on your computer: no network access, no accounts.

**Status:** early development (0.1.0 released). You can create decks, add plain-text cards and study them with Fail and Pass on an FSRS schedule. Markdown and math rendering are not there yet. See [SPEC.md](SPEC.md) for what is planned.

## Building

You need:

- Rust 1.92 or newer (`rustup`)
- GTK ≥ 4.14 and libadwaita ≥ 1.6 development files
  (Fedora: `gtk4-devel libadwaita-devel`; Debian/Ubuntu: `libgtk-4-dev libadwaita-1-dev`)
- `blueprint-compiler` and `glib-compile-resources` (from GLib) on your `PATH`
- [`cargo-deny`](https://github.com/EmbarkStudios/cargo-deny) for the quality gate (`cargo install cargo-deny`)

```sh
cargo run -p cards                         # run the app
RUST_LOG=cards=debug cargo run -p cards    # with debug logs
GTK_DEBUG=interactive cargo run -p cards   # with the GTK inspector
scripts/check.sh                           # format, lint, test, licence/advisory checks
scripts/install-hooks.sh                   # once per clone: run checks before every commit
```

To try the app on scratch data instead of your own (60 cards, 30 of them due now):

```sh
cargo run -p cards-core --example fill_scratch_db -- /tmp/cards-scratch 60 --due 30
CARDS_DATA_DIR=/tmp/cards-scratch cargo run -p cards
```

## Recovering your data

Cards backs up its database automatically: before every schema upgrade, before an import that adds anything, and once a day, into `backups/` inside the data directory (`~/.local/share/cards/backups/`, keeping the 14 newest).

If the database is damaged or you want to go back to an earlier state, **restore a backup through Cards. Do not copy files by hand.** A copied-in file can be silently mixed with a leftover `cards.sqlite3-wal` file from the old database, which corrupts it or brings back data you meant to drop. Restoring checks the backup first and refuses a damaged one. It never deletes your current database: it is kept next to the new one as `cards.sqlite3.replaced-YYYYMMDD-HHMMSS` (with its `-wal`/`-shm`/`-journal` files, if any).

When Cards finds its database damaged, it shows a recovery screen that lists the backups, newest first, and restores the one you pick. Restoring a backup while the database is healthy (to go back to an earlier state) is not in the app yet. If a database was made by a newer version of Cards, update Cards; do not restore an older backup over it.

## Exporting and importing

**Export…** in the main menu (☰ on the deck list, or F10) saves everything (decks, cards and the whole review history, deleted items included) to one `.cards` file. Keep it as a copy, or open it on another computer with **Import…**.

Import first shows what the file would add and what is already there, and writes nothing until you confirm. It only ever adds: nothing you have is changed or deleted. If the file disagrees with your data (for example a card edited in both places), nothing is imported and Cards lists where they differ. Importing the same file twice changes nothing.

## Documentation

- [SPEC.md](SPEC.md): what to build.
- [ARCHITECTURE.md](ARCHITECTURE.md): how the code is organised.
- [docs/adr/](docs/adr/): architecture decision records.
- [CHANGELOG.md](CHANGELOG.md): notable changes.

## Licence

GPL-3.0-or-later. See [LICENSE](LICENSE).
