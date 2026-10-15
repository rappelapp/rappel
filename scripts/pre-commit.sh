#!/usr/bin/env bash
# Git pre-commit hook: refuse commits on main, then run the quality gate.
# Install with scripts/install-hooks.sh. Never bypass with --no-verify.
# Note: the gate checks the working tree, not only the staged changes, so
# commit with a clean tree (no unstaged edits) for the result to be meaningful.
set -euo pipefail

branch="$(git symbolic-ref --quiet --short HEAD || true)"
if [[ "$branch" == "main" ]]; then
    echo "pre-commit: refusing to commit on 'main'. Create a slice branch first," >&2
    echo "            e.g. git switch -c feat/my-slice" >&2
    exit 1
fi
if [[ -z "$branch" ]]; then
    echo "pre-commit: refusing to commit on a detached HEAD. Switch to a slice branch." >&2
    exit 1
fi

exec "$(git rev-parse --show-toplevel)/scripts/check.sh"
