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

`src/lib.rs` only: the app name and ID constants and `is_valid_app_id`, a placeholder with unit tests so the test pipeline has something real to run.

### `cards` today

- `src/main.rs`: sets up `tracing` (level from `RUST_LOG`), registers the embedded gresource, creates the `adw::Application` and shows the main window.
- `src/window.rs`: `CardsWindow`, an `adw::ApplicationWindow` subclass using a composite template.
- `data/window.blp`: the window layout in Blueprint (header bar + empty `AdwStatusPage`).

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
