# Cards

A native GNOME flashcard app. Two grades only (Fail and Pass), Markdown cards with LaTeX math, FSRS scheduling. Everything stays on your computer: no network access, no accounts.

**Status:** early development. Nothing usable yet; v0 has not started. See [SPEC.md](SPEC.md) for what is planned.

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

## Recovering your data

Cards backs up its database automatically: before every schema upgrade and once a day, into `backups/` inside the data directory (`~/.local/share/cards/backups/`, keeping the 14 newest).

If the database is damaged or you want to go back to an earlier state, **restore a backup through Cards. Do not copy files by hand.** A copied-in file can be silently mixed with a leftover `cards.sqlite3-wal` file from the old database, which corrupts it or brings back data you meant to drop. Restoring checks the backup first and refuses a damaged one. It never deletes your current database: it is kept next to the new one as `cards.sqlite3.replaced-YYYYMMDD-HHMMSS` (with its `-wal`/`-shm`/`-journal` files, if any).

The restore screen is not in the app yet, so for now there is no supported way to restore. If Cards refuses to open your database, leave the files in the data directory exactly as they are until the restore screen arrives.

## Documentation

- [SPEC.md](SPEC.md): what to build.
- [ARCHITECTURE.md](ARCHITECTURE.md): how the code is organised.
- [docs/adr/](docs/adr/): architecture decision records.
- [CHANGELOG.md](CHANGELOG.md): notable changes.

## Licence

GPL-3.0-or-later. See [LICENSE](LICENSE).
