# Vendored: ratex-cairo 0.1.15

Upstream: https://github.com/erweixin/RaTeX (crates.io `ratex-cairo` 0.1.15), MIT, see `LICENSE`.

Changes from the published crate (ADR 0021):

- `Cargo.toml`: `cairo-rs` `0.20` → `0.22`. Rappel is on gtk4 0.11 / cairo-rs 0.22, and
  both `cairo-sys-rs` versions declare `links = "cairo"`, so Cargo cannot build the two together.
- `Cargo.toml`: an empty `[workspace]` table, so the crate can be tested on its own
  (`cargo test --manifest-path third_party/ratex-cairo/Cargo.toml`). The Rappel workspace
  excludes `third_party`, so fmt, clippy and tests of the gate skip this code.
- `src/lib.rs` is unchanged. `Cargo.toml.orig` is upstream's, untouched.

Used through `[patch.crates-io]` in the root `Cargo.toml` (a path, not git: `deny.toml` has
`allow-git = []`).

Remove this directory and the patch once upstream publishes a `ratex-cairo` on our gtk-rs
versions. Check at every `ratex-*` bump.
