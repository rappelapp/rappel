#!/usr/bin/env bash
# Reads cargo's JSON messages (`cargo build --message-format=json…`) on
# stdin and prints, for the package PACKAGE_ID, two lines: the binary cargo
# built, then its build script's output folder (OUT_DIR). Used by
# scripts/snap-install.sh (ADR 0085, ADR 0090).
#
#   cargo-outputs.sh PACKAGE_ID < messages
#
# PACKAGE_ID is what `cargo pkgid -p <package>` prints, which from cargo
# 1.77 on is the `package_id` of that package's messages
# (`path+file:///…/crates/app#rappel@0.4.3`); older cargo is not supported. The
# messages are matched by it, never by folder names: the build folder's
# layout changed in cargo 1.100 (`build/rappel-<hash>/out` became
# `build/rappel/<hash>/out`) and may change again.
#
# Exits 1, printing nothing, unless exactly one binary and exactly one
# output folder are found. Cargo prints one JSON object per line, and paths
# with a `"` or `\` (escaped in JSON) are not supported.
set -euo pipefail

if [ "$#" -ne 1 ] || [ -z "$1" ]; then
    echo "usage: $0 PACKAGE_ID < cargo-json-messages" >&2
    exit 2
fi
package_id=$1

# The package's own messages: its id in full, quotes included, so no other
# package whose id starts the same way matches. The comma relies on cargo's
# compact JSON with a field after `package_id` (true of both messages used);
# if that ever changes, nothing matches and the script fails, never guesses.
mine=$(grep -F "\"package_id\":\"$package_id\"," || true)

# `executable` is null for a library or the build script, and a path for a
# binary (cargo's docs, "JSON messages", artifact messages).
binaries=$(grep -F '"reason":"compiler-artifact"' <<<"$mine" \
    | sed -n 's|.*"executable":"\([^"]*\)".*|\1|p' | sort -u || true)
out_dirs=$(grep -F '"reason":"build-script-executed"' <<<"$mine" \
    | sed -n 's|.*"out_dir":"\([^"]*\)".*|\1|p' | sort -u || true)

count() { grep -c . <<<"$1" || true; }
if [ "$(count "$binaries")" -ne 1 ] || [ "$(count "$out_dirs")" -ne 1 ]; then
    echo "$0: cargo reported $(count "$binaries") binaries and $(count "$out_dirs")" \
        "build-script folders for $package_id; expected one of each" >&2
    exit 1
fi
printf '%s\n%s\n' "$binaries" "$out_dirs"
