# 3. Continuous integration on GitHub Actions

- Status: Accepted
- Date: 2026-10-15

## Context

`scripts/check.sh` is the single quality gate, but until now only the local pre-commit hook ran it. PRs need an independent run that can be marked as a required status check before merging into `main`.

## Decision

- One workflow, `.github/workflows/ci.yml`, with one job `check` that runs `scripts/check.sh` unchanged. It runs on every pull request and on pushes to `main`. CI never has its own, weaker list of checks.
- The job runs in an **`ubuntu:26.04` container** and installs the system packages listed in README.md, instead of using the packages that happen to be on GitHub's runner image. CI therefore tests the documented build instructions, and a change to the runner image cannot silently change our GTK, libadwaita or blueprint-compiler versions.
- Rust is installed with the official `rustup` script on the **stable** toolchain with `rustfmt` and `clippy`, matching local development.
- Caching uses `Swatinem/rust-cache` (cargo registry, git checkouts and `target/`, keyed on `Cargo.lock` and the compiler version).
- `cargo-deny` comes from `taiki-e/install-action` as a checksum-verified prebuilt binary, with the build-from-source fallback disabled.
- Every third-party action is pinned to a full commit SHA with the version in a comment. The workflow token has read-only `contents` permission.

## Consequences

- A new stable Rust release can add clippy lints that fail CI on unchanged code. That is fixed in a small `fix/` or `chore/` slice; we do not pin an old toolchain to avoid it.
- CI builds against the GTK/libadwaita in Ubuntu 26.04, which is newer than our declared minimum (ADR 0002). CI does not prove the minimum versions or the Rust MSRV (1.92) still build; a separate job can do that later if needed.
- `cargo deny` downloads the RustSec advisory database in CI. This is network access by the build tooling, not by the app, so the "no network" rule is unaffected.
- Pinned actions need occasional manual updates (or Dependabot later).
