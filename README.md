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

## Documentation

- [SPEC.md](SPEC.md): what to build.
- [ARCHITECTURE.md](ARCHITECTURE.md): how the code is organised.
- [docs/adr/](docs/adr/): architecture decision records.
- [CHANGELOG.md](CHANGELOG.md): notable changes.

## Licence

GPL-3.0-or-later. See [LICENSE](LICENSE).
