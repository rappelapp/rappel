#!/usr/bin/env bash
# Installs scripts/pre-commit.sh as this clone's git pre-commit hook.
# Safe to run more than once.
set -euo pipefail

root="$(git -C "$(dirname "$0")" rev-parse --show-toplevel)"
hooks_dir="$(git -C "$root" rev-parse --path-format=absolute --git-path hooks)"
hook="$hooks_dir/pre-commit"

mkdir -p "$hooks_dir"
if [[ -e "$hook" && ! -L "$hook" ]]; then
    backup="$hook.bak.$(date +%Y%m%d-%H%M%S)"
    echo "install-hooks: $hook exists and is not our symlink; moving it to $backup" >&2
    mv "$hook" "$backup"
fi
ln -sfn "$root/scripts/pre-commit.sh" "$hook"
echo "install-hooks: pre-commit hook installed -> scripts/pre-commit.sh"
