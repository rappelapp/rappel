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

## Amendment (2026-11-02): docs-only pull requests skip the gate

At the maintainer's request, to stop spending CI time on changes the gate cannot check.

- A first job, `changes`, lists the files a pull request changes (its merge commit against the base, `fetch-depth: 2`). The pull request is **docs-only** when every file is under `docs/` or `.claude/`, or is a Markdown file at the top level (`SPEC.md`, `ARCHITECTURE.md`, `CLAUDE.md`, `README.md`), **except `CHANGELOG.md`**, which the tests read against the AppStream metainfo (`crates/app/src/desktop_files.rs`).
- `check` needs `changes` and is skipped for a docs-only pull request. GitHub counts a job skipped by its `if` as a success for a required status check, so the ruleset still lets the pull request merge.
- Fail-safe: `check` runs whenever `changes` did not succeed, and for every push to `main`. An empty file list runs it too. The gate itself is unchanged.
- Consequence: a test that starts reading a file under `docs/`, `.claude/` or a top-level Markdown file must add it to the exception in `ci.yml`, as `CHANGELOG.md` is.
- Corrected the same day (the review of #111): the list is taken with `--no-renames`, so a rename lists its old path too (moving a source file into `docs/` is not docs-only), and the classification pipes nothing into `grep -q`, which under GitHub's `bash -eo pipefail` could make the writer die of `SIGPIPE` and read a long list of code files as docs-only.
