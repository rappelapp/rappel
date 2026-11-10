# 85. The snap: packaging

- Status: Accepted
- Date: 2026-11-10
- Refers to: `docs/briefs/phase-3b-snap.md` (slice 2); ADR 0082 (feasibility and the packaging plan), ADR 0055 and ADR 0082 amendments (the image loader in the snap), ADR 0059 (`RAPPEL_LOCALEDIR`, the `.mo` files from `build.rs`), ADR 0060 (`data/`), ADR 0062 (GStreamer allow-list), ADR 0003 (CI)

## Context

ADR 0082 checked that a strictly confined snap on `core26` with the `gnome` extension can run Rappel, and wrote a plan for packaging. Its one blocker, the image sandbox, was answered by the maintainer (option A) and built in slice 1. This slice packages Rappel as that plan says. The brief pre-approves `snap/snapcraft.yaml`, `scripts/snap-install.sh` and `.github/workflows/snap.yml`, the desktop plugs and `audio-playback`, the `dbus` slot for the app id, and staging `faad` for M4A only if its licence is allowed. It stops at a `home`, `removable-media`, `network` or `browser-support` plug, classic confinement, a store token or secret, and a new Cargo dependency.

## Decision

### 1. Installing without Meson: `build.rs` prepares, `scripts/snap-install.sh` copies

- **`build.rs`** already compiled `po/<lang>.po` into `$OUT_DIR/locale/<lang>/LC_MESSAGES/rappel.mo` (ADR 0059). It now also merges every language of `po/LINGUAS` into `data/io.github.rappelapp.Rappel.desktop.in` and `….metainfo.xml.in` and writes them without `.in` to `$OUT_DIR/data/`, with the same code the tests use to validate them (`i18n/extract.rs`, included into the build script by `#[path]` as `po.rs` already was). So packaging needs no `msgfmt`, no Meson and no gettext tools, and what is installed is exactly what the tests validate.
- **Several languages in one pass** (`merge_desktop_languages`, `merge_metainfo_languages`; the old one-language functions call them): a merged file is no longer a source (`Name[fr]=` in a source is refused), so merging language by language would fail at the second.
- **`scripts/snap-install.sh DESTDIR [PREFIX]`** (PREFIX defaults to `/usr`) builds the release binary (`cargo build --release --locked -p rappel`), reads from cargo's JSON messages where the binary and the build script's output folder are (its hash cannot be guessed), and installs: `bin/rappel` (0755), each `.mo` (0644), the desktop entry under `share/applications/`, the metainfo under `share/metainfo/`, and the icons of `data/icons/` under `share/icons/` as they are. It refuses a relative path, `..`, or a number of `.mo` files that differs from `po/LINGUAS`.
- **The test** (`crates/app/src/snap_install.rs`, in the gate) runs the script with two test variables, `RAPPEL_BINARY` (a stand-in file) and `RAPPEL_OUT_DIR` (the test build's own `OUT_DIR`, which `build.rs` filled), so it needs no release build: it checks the exact list of installed files and their modes against the app id, the binary's and the domain's names, each file byte for byte against its source, that the installed desktop entry and metainfo pass `desktop-file-validate` and `appstreamcli validate --no-net --pedantic` with every language merged, and that bad arguments are refused with nothing written. Nothing in `scripts/check.sh` changed.

### 2. `snap/snapcraft.yaml`

- `name: rappel`, `base: core26`, the `gnome` extension (platform `gnome-core26`: GTK 4.24, libadwaita 1.10, GtkSourceView 5.22; ADR 0082, finding 1), `confinement: strict`, `grade: stable` (QUESTIONS.md, Phase 3b, question 4), `license: GPL-3.0-or-later`, `platforms: amd64` only (what CI builds; arm64 later if asked).
- **Metadata:** `adopt-info` with `parse-info` on the installed metainfo gives the title, summary and description; the version is set from `Cargo.toml`'s `[workspace.package]` with `craftctl set version=` (which takes precedence over the metainfo's newest release, snapcraft's `update_project_variables`). The icon is the app icon in `data/icons/` (`icon:`), and `common-id: io.github.rappelapp.Rappel` with `desktop:` keeps the desktop file's and metainfo's names.
- **Build:** the `rust` plugin only for its toolchain (rustup, `rust-channel: stable`, as CI's gate uses), with `override-build` running `scripts/snap-install.sh "$CRAFT_PART_INSTALL" /usr`. The GNOME SDK (`gnome-core26-sdk`, added by the extension) provides the `-dev` libraries, `blueprint-compiler` and `glib-compile-resources`; nothing is staged.
- **Permissions:** the app's plugs are the extension's (`desktop`, `desktop-legacy`, `opengl`, `wayland`, `x11`, and the content plugs for the platform, themes and GPU) plus `audio-playback`. One slot: `dbus`, `bus: session`, `name: io.github.rappelapp.Rappel`, so GApplication can own its id (single instance; ADR 0082, "Also found"; question 2). No `home`, `removable-media`, `network` or `browser-support`; the test reads the file and fails on any of them, on `classic` or `devmode`, and on any list item but the three it expects.
- **Environment:** `RAPPEL_LOCALEDIR=$SNAP/usr/share/locale` (the base's `/usr/share/locale` holds no Rappel catalog; ADR 0059). `RAPPEL_DATA_DIR` is not set: the collection is per revision, in `~/snap/rappel/<revision>/.local/share/rappel` (question 3; README).

### 3. M4A is not supported in the snap

The platform has every plugin of the GStreamer allow-list but an AAC decoder (ADR 0082, finding 3). The one that could be staged, `faad` from `gstreamer1.0-plugins-bad`, links `libfaad2`, whose licence is **GPL-2.0-or-later** (`/usr/share/doc/libfaad2/copyright`: `GPL-2+`). That licence is compatible with GPL-3.0-or-later, but it is not in `deny.toml`'s allow-list, and the brief stages `faad` only "if its licence is allowed (check `deny.toml`…)", while CLAUDE.md treats a dependency with a licence not already allowed as a stop condition. So nothing is staged: in the snap an M4A file is "could not be played", as on any system without an AAC decoder (ADR 0062), and the README says so. MP3, Ogg Opus, Ogg Vorbis and WAV play. Adding GPL-2.0-or-later to the allow-list and staging `faad` (with a `prime` filter keeping `libgstfaad.so` and `libfaad2.so*`) is left to the maintainer (QUESTIONS.md, Phase 3b part 2).

### 4. `.github/workflows/snap.yml`: builds, never publishes

- Triggers: `v*` tags, `workflow_dispatch`, and pull requests that touch `snap/**`, `scripts/snap-install.sh` or the workflow itself, so a pull request that changes packaging proves the snap still builds. Not on other pull requests (the build takes much longer than the gate, and `check` is the gate) and so never on a docs-only one. This revises ADR 0082's plan ("never on pull requests"), as the brief asks.
- `permissions: contents: read`, checkout without persisted credentials, `snapcore/action-build` pinned by commit SHA (v1.3.0, the latest release; it runs snapcraft from the stable channel in LXD), and `actions/upload-artifact` (pinned) keeping the `.snap` as a workflow artifact. No store login, token or secret; the test fails on `secrets`, `upload` (but the artifact action), `publish` or store credentials in the file, and on any action not pinned by a full SHA.
- The `ci.yml` gate still runs for such pull requests (they change code paths), so `check` stays the required status.

### 5. Carry-over from slice 1: devmode

`loader_sandbox::choose` recognises the snap by its environment and the binary's path, not by its confinement. A snap installed with `--devmode` (or run on a distribution without AppArmor, where snapd confines only partly) therefore also runs the loader without bubblewrap, with no real boundary around it. Requiring AppArmor enforce mode (`/proc/self/attr/current` reading `snap.rappel.rappel (enforce)`) was considered and not done: it is not testable here without a real snap, and on a system without AppArmor it would bring back `Auto`, which inside a snap makes every image fail or fall back silently (ADR 0082, finding 2). Documented in the ADR 0055 and 0082 amendments; `--devmode` is a developer's choice and the README installs with `--dangerous` only.

## Consequences

- No new Cargo dependency, no migration, no export-format change. `build.rs` compiles `extract.rs` too (a second or so of build time; its unused parts are `#[expect(dead_code)]`).
- The snap was **not built on the development machine**: there is no LXD, and `--destructive-mode` refuses a 26.10 host for a `core26` (26.04) build. `snapcraft expand-extensions` accepted the file. The pull request's `snap` job is the proof that it builds.
- Not verified on a real snap (no root to install one): the `dbus` slot, the portal folder's persistence, the start-up log line, French with `LANGUAGE=fr`, and images in the snap. The maintainer's checklist (the PR's **Try it**; slice 3's report) covers them.
- The version is not bumped here (slice 3 makes 0.4.1).
