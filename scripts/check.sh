#!/usr/bin/env bash
# The single quality gate. A slice is not done until this passes.
# Runs as the git pre-commit hook (scripts/pre-commit.sh) and in CI (.github/workflows/ci.yml).
set -euo pipefail

cd "$(git -C "$(dirname "$0")" rev-parse --show-toplevel)"

step() {
    printf '\n\033[1m==> %s\033[0m\n' "$*"
}

step "cargo fmt --check"
cargo fmt --all --check

step "cargo clippy (warnings are errors)"
cargo clippy --workspace --all-targets --locked -- -D warnings

step "cargo test"
cargo test --workspace --locked

step "cargo deny check"
cargo deny --locked check --hide-inclusion-graph

printf '\n\033[1;32mAll checks passed.\033[0m\n'
