#!/usr/bin/env bash
# Installs Rappel into DESTDIR under PREFIX (default /usr), without Meson
# (ADR 0082, plan; ADR 0085). snap/snapcraft.yaml runs it as
#   scripts/snap-install.sh "$CRAFT_PART_INSTALL" /usr
#
# It builds the release binary with cargo, then installs:
#   PREFIX/bin/rappel
#   PREFIX/share/locale/<lang>/LC_MESSAGES/rappel.mo   (build.rs compiles them)
#   PREFIX/share/applications/<app id>.desktop         (build.rs merges the
#   PREFIX/share/metainfo/<app id>.metainfo.xml         translations into both)
#   PREFIX/share/icons/hicolor/...                     (data/icons, as they are)
#
# The test (crates/app/src/snap_install.rs) skips the release build: it sets
# RAPPEL_BINARY (a file to install as the binary) and RAPPEL_OUT_DIR (the
# app crate's build-script output folder, holding locale/ and data/). Both
# or neither.
set -euo pipefail

usage() {
    echo "usage: $0 DESTDIR [PREFIX]" >&2
    echo "  DESTDIR: an absolute folder; PREFIX: an absolute path, default /usr" >&2
    exit 2
}

[ "$#" -ge 1 ] && [ "$#" -le 2 ] || usage
destdir=$1
prefix=${2:-/usr}
case $destdir in /*) ;; *) usage ;; esac
case $prefix in /*) ;; *) usage ;; esac
# No `..` anywhere: the install must stay inside DESTDIR.
case "/$destdir/$prefix/" in */../*) usage ;; esac

# The repository is the folder above this script (as in check.sh).
repo=$(dirname "$(realpath "$0")")/..
cd "$repo"

app_id=io.github.rappelapp.Rappel
# The binary target, the gettext domain and the package are all `rappel`.
name=rappel

if [ -n "${RAPPEL_BINARY:-}" ] || [ -n "${RAPPEL_OUT_DIR:-}" ]; then
    if [ -z "${RAPPEL_BINARY:-}" ] || [ -z "${RAPPEL_OUT_DIR:-}" ]; then
        echo "$0: set both RAPPEL_BINARY and RAPPEL_OUT_DIR, or neither" >&2
        exit 2
    fi
    binary=$RAPPEL_BINARY
    out_dir=$RAPPEL_OUT_DIR
else
    # Cargo's JSON messages name the binary it built and the build script's
    # output folder, whose hash cannot be guessed: `.../build/rappel-<hex>/out`
    # (rappel-core's folder is `rappel-core-<hex>`, which `[0-9a-f]` refuses).
    messages=$(cargo build --release --locked -p "$name" --message-format=json-render-diagnostics)
    binary=$(grep '"reason":"compiler-artifact"' <<<"$messages" \
        | sed -n "s|.*\"executable\":\"\([^\"]*/$name\)\".*|\1|p" | tail -n 1)
    out_dir=$(grep '"reason":"build-script-executed"' <<<"$messages" \
        | sed -n "s|.*\"out_dir\":\"\([^\"]*/build/$name-[0-9a-f]*/out\)\".*|\1|p" | tail -n 1)
    if [ -z "$binary" ] || [ -z "$out_dir" ]; then
        echo "$0: cargo did not report the binary or the build script's folder" >&2
        exit 1
    fi
fi

[ -f "$binary" ] || { echo "$0: no binary at $binary" >&2; exit 1; }
for file in "data/$app_id.desktop" "data/$app_id.metainfo.xml"; do
    [ -f "$out_dir/$file" ] || { echo "$0: $out_dir/$file is missing (build.rs makes it)" >&2; exit 1; }
done

root=$destdir$prefix
install -D -m 0755 "$binary" "$root/bin/$name"

found=0
for mo in "$out_dir"/locale/*/LC_MESSAGES/"$name".mo; do
    [ -f "$mo" ] || continue
    language=$(basename "$(dirname "$(dirname "$mo")")")
    install -D -m 0644 "$mo" "$root/share/locale/$language/LC_MESSAGES/$name.mo"
    found=$((found + 1))
done
# po/LINGUAS lists the languages; each must have been compiled.
expected=$(grep -c -v -E '^[[:space:]]*(#|$)' po/LINGUAS || true)
if [ "$found" -ne "$expected" ]; then
    echo "$0: $found catalogs in $out_dir/locale, po/LINGUAS lists $expected" >&2
    exit 1
fi

install -D -m 0644 "$out_dir/data/$app_id.desktop" "$root/share/applications/$app_id.desktop"
install -D -m 0644 "$out_dir/data/$app_id.metainfo.xml" "$root/share/metainfo/$app_id.metainfo.xml"

# The icons as they are in data/icons (hicolor/<size>/apps/<name>.svg).
(cd data/icons && find . -type f -name '*.svg' -print0) | while IFS= read -r -d '' icon; do
    install -D -m 0644 "data/icons/$icon" "$root/share/icons/${icon#./}"
done

echo "Installed Rappel into $root"
