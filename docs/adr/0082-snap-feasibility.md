# 82. The snap: feasibility check, stopped at the image sandbox

- Status: Accepted (findings); packaging was blocked on a maintainer decision (`docs/briefs/QUESTIONS.md`, Phase 3b, question 1), answered on 2026-11-10 (amendment below)
- Date: 2026-11-09
- Refers to: Phase 3b brief, slice 7 and its stop conditions; ADR 0055 (glycin's sandbox), ADR 0056 and 0062 (GStreamer allow-list), ADR 0059 (`RAPPEL_LOCALEDIR`), ADR 0060 (`data/`), ADR 0004 (data directory), ADR 0080 (backup folder through the portal)

## Context

Slice 7 packages Rappel as a strictly confined snap with the `gnome` extension. The brief asks for a feasibility check first, written as this ADR, and stops the slice if the snap cannot give libadwaita ≥ 1.8 and GtkSourceView 5, or if glycin's image sandbox (bubblewrap) does not run under strict confinement. Weakening the image sandbox is the maintainer's decision.

How it was checked, on the development machine (Ubuntu 26.10, snapd 2.77.1, snapcraft 9.2.1, kernel with `kernel.apparmor_restrict_unprivileged_userns = 1`):

- snapcraft's own `gnome` extension source (`/snap/snapcraft/current/lib/python3*/site-packages/snapcraft/extensions/gnome.py`) and its launcher scripts (`/snap/snapcraft/current/share/snapcraft/extensions/desktop/`).
- The GNOME platform and SDK snaps downloaded with `snap download` (no install, no root): `gnome-core26` revision 33 and `gnome-core26-sdk` revision 25, both `latest/stable` of 2026-10-09, listed with `unsquashfs -l`; the SDK's `pkg-config` files and its build recipe (`snap/snapcraft.yaml` inside the snap).
- Commands run inside two strict snaps already installed here (`snap run --shell`): `tree` (core22) and `snap-store` (core24, `gnome` extension, with the desktop plugs and more).
- Upstream sources, fetched for reading only: the GNOME SDK's glycin patch, snapd's interface policies, xdg-desktop-portal's document code (Sources, below).

No snap of Rappel was built or installed: `snap install --dangerous` needs root, which this run does not have, and the slice stopped before packaging anyway.

## Findings

### 1. libadwaita, GTK 4 and GtkSourceView 5: available (high confidence)

snapcraft 9.2.1's `gnome` extension supports `core22`, `core24` and `core26`. For `core26` it builds against `gnome-core26-sdk` and runs on the content snap `gnome-core26` (mounted at `$SNAP/gnome-platform`); the older names (`gnome-46-2404`) are core24's. There is no `gnome-5x-2604` snap: the core26 platform drops the GNOME version from its name.

Versions in `gnome-core26-sdk` 25 (`usr/lib/x86_64-linux-gnu/pkgconfig/*.pc`), with the libraries present in `gnome-core26` 33:

| Library | In the snap | Rappel needs (`Cargo.toml` features) |
|---|---|---|
| libadwaita | 1.10.0 | ≥ 1.8 (`libadwaita` feature `v1_8`; `AdwToggleGroup` is 1.7) |
| GTK 4 | 4.24.0 | ≥ 4.14 (`gtk4` feature `v4_14`; list section headers are 4.12) |
| GtkSourceView 5 | 5.22.0 | ≥ 5.10 (`sourceview5` feature `v5_10`) |
| GLib | 2.90.0 | (through the above) |
| GStreamer | 1.28.2 | 1.28 (ADR 0062 was checked with 1.28) |
| glycin (C library and loaders) | 2.2.1 | loaders 2.x (`glycin` crate 3.1 talks to `glycin-loaders/2+`) |

The same versions as the development machine (libadwaita 1.10.0, GtkSourceView 5.22.0) and as CI's `ubuntu:26.04`. Rappel's other run-time libraries (lcms2, libseccomp, fontconfig, graphene, cairo) are in `gnome-core26` too; the GL, Wayland and X libraries come from the base and the GPU content snap the extension adds. **No stop condition here.**

### 2. glycin's bubblewrap sandbox: does not run under strict confinement (high confidence) — STOP

Three independent pieces of evidence:

1. **Canonical disables it in its own platform.** `gnome-core26-sdk` builds glycin 2.2.1 with `patches/glycin-snap-no-sandbox.patch` (Alessandro Astone, Canonical, 7 July 2026): "We cannot run bwrap within snap confinement, but we can still set a seccomp filter at least." The patch adds a `RunEnvironment::Snap` (any non-empty `$SNAP`) that selects `NotSandboxed` and then starts the loader as a plain child with a seccomp filter and `PR_SET_NO_NEW_PRIVS`. That is GNOME's C library in the platform snap; Rappel does not use it (below).
2. **User namespaces are refused.** `unshare(CLONE_NEWUSER)` (syscall 272, called from Perl) returns `EPERM` ("Operation not permitted") inside both `tree` and `snap-store`, and succeeds on the host outside any snap. snapd grants `userns` in AppArmor only through `browser-support` with `allow-sandbox: true` (`interfaces/builtin/browser_support.go`, `browserSupportConnectedPlugAppArmorWithSandboxUserNS`), an interface for web browsers, outside the brief's plug set, which also would not give bubblewrap the `mount` and `pivot_root` it needs.
3. **bubblewrap itself fails earlier.** Inside `snap-store` (core24, the `gnome` extension's environment, more plugs than Rappel would have), the platform's own `bwrap --unshare-all --die-with-parent --ro-bind /usr /usr --dev /dev /usr/bin/true` stops at once: `bwrap: Can't read /proc/sys/kernel/overflowuid: Permission denied` (an AppArmor denial), exit status 1.

**What Rappel would do in the snap today.** Rappel uses the `glycin` crate 3.1 (the Rust implementation, linked into the binary), not the platform's patched C library, with `SandboxSelector::Auto` (ADR 0055). The launcher puts `$SNAP/gnome-platform/usr/bin` on `PATH` (`desktop-exports`: `append_dir PATH "$SNAP_DESKTOP_RUNTIME/usr/bin"`), so `bwrap` is found. glycin 3.1 decides between `Bwrap` and `NotSandboxed` with a test run of bwrap (`check_bwrap_syscalls_blocked`, `sandbox.rs`): only `SIGSYS` or one of five known messages ("No permissions to create a new namespace", "Creating new namespace failed", …) count as "blocked"; any other failure is taken as "bwrap works". The message seen above is not one of them, so glycin would choose `Bwrap`, every decode would fail, and **every image would show as "could not be shown"**. If a later bubblewrap reached `unshare` first, the `EPERM` message would match, glycin would fall back to `NotSandboxed` (a plain child: no seccomp, no namespaces, no memory limit) and Rappel would log its one warning: images would work, **silently without the sandbox**. Neither is acceptable without a decision; glycin 4.0.3 (newest on crates.io here) has no snap handling either.

**What strict confinement gives instead.** Without bubblewrap, the loader still runs inside the snap's own AppArmor and seccomp profile, like Rappel itself: with the planned plugs it has no network (no `network` plug) and no home folder (no `home`). It could still read and change what Rappel can: the data folder (database, media, `auto-backup.ini`), the folders granted through the portal (the backup folder), and the session bus within the desktop interfaces. With bubblewrap it can reach none of these. That difference is what the maintainer is asked to weigh (QUESTIONS.md, Phase 3b, question 1).

**Stop condition hit:** the slice stops here. Nothing is packaged, and the image code is unchanged.

### 3. The GStreamer allow-list: 17 of 21 in the platform; AAC in M4A needs one staged plugin (high confidence for presence; playback not run)

The launcher sets `GST_PLUGIN_SYSTEM_PATH=$SNAP_DESKTOP_RUNTIME/usr/lib/$ARCH/gstreamer-1.0` and `GST_PLUGIN_PATH=$SNAP/usr/lib/$ARCH/gstreamer-1.0`, so plugins come from the platform and from Rappel's own snap. Rappel's allow-list names plugins, not paths (`gst_env.rs`), so it applies unchanged. In `gnome-core26` 33 (`usr/lib/x86_64-linux-gnu/gstreamer-1.0/`, 108 plugins):

| Plugin | In `gnome-core26` | Ubuntu package | Effect if missing |
|---|---|---|---|
| `coreelements` | yes | libgstreamer1.0-0 | |
| `typefindfunctions`, `playback`, `gio`, `audioconvert`, `audioresample`, `volume`, `ogg`, `opus`, `vorbis` | yes | gstreamer1.0-plugins-base | |
| `autodetect`, `pulseaudio`, `wavparse`, `audioparsers`, `mpg123`, `isomp4` | yes | gstreamer1.0-plugins-good | |
| `pipewire` | yes | gstreamer1.0-pipewire | |
| `alsa` | **no** | gstreamer1.0-alsa | None: `autoaudiosink` picks `pulsesink` (primary rank), which the `audio-playback` plug serves. |
| `opusparse` | **no** | gstreamer1.0-plugins-bad | None: its rank is `none`, so playback never plugs it; Ogg Opus goes `oggdemux` → `opusdec`. |
| `fdkaac` | **no** | gstreamer1.0-fdkaac (multiverse) | Either AAC decoder is enough. |
| `faad` | **no** | gstreamer1.0-plugins-bad | **No AAC decoder: an M4A (AAC) does not play** ("could not be played", as on a system without it, ADR 0062). |

So MP3, Ogg Opus, Ogg Vorbis and WAV are covered by the platform. For M4A, the snap would stage `libgstfaad.so` and `libfaad2` from the core26 archive (`stage-packages: [gstreamer1.0-plugins-bad]` with a `prime` filter keeping only those two), or accept that M4A does not play in the snap. Not a stop condition; a choice for the packaging step.

### 4. The data directory: `~/snap/rappel/<revision>/.local/share/rappel` (high confidence)

The launcher exports `XDG_DATA_HOME="$SNAP_USER_DATA/.local/share"` and `XDG_CACHE_HOME="$SNAP_USER_COMMON/.cache"` (`desktop-exports`, lines 143 and 151), and snapd sets `HOME=$SNAP_USER_DATA`. Rappel's release build resolves `$XDG_DATA_HOME/rappel` (`rappel_core::data_dir::resolve_data_dir`, ADR 0004), so the data lands in `~/snap/rappel/<revision>/.local/share/rappel`, reachable as `~/snap/rappel/current/…`. Consequences:

- **The snap starts empty.** It cannot see `~/.local/share/rappel` (no `home` plug). The user moves the collection with Export (from the other install) and Import (in the snap, through the portal), or copies the folder once by hand, after starting and closing the snap once (so that `~/snap/rappel/current/.local/share/` exists) and with it closed: `cp -a ~/.local/share/rappel ~/snap/rappel/current/.local/share/`.
- **`$SNAP_USER_DATA` is per revision.** snapd copies it on each refresh and keeps the copies of the retained revisions (`refresh.retain`, 2 by default on desktops), so `snap revert` brings back the data as it was copied at that refresh, from before a migration. **But a revert also hides every review and edit made since that refresh:** they stay only in the newer revision's folder, which snapd deletes when that revision is dropped. The other cost is disk space: the media folder is copied too, once per kept revision. The alternative is `RAPPEL_DATA_DIR=$SNAP_USER_COMMON/rappel` (one copy, shared across revisions, no rollback of data). A choice for the maintainer (QUESTIONS.md, question 3).
- GStreamer's registry (ADR 0062) goes to `$XDG_CACHE_HOME`, in `$SNAP_USER_COMMON`: rebuilt in about 0.1 s if missing.
- Translations: inside the snap `/usr/share/locale` is the base's, so the snap must set `RAPPEL_LOCALEDIR=$SNAP/usr/share/locale` (read at run time, ADR 0059).

### 5. Folders chosen through the portal: persistent by design (medium confidence; to confirm on a real snap)

The `gnome` extension sets `GTK_USE_PORTAL=1`, and GTK uses the FileChooser portal in a snap anyway. For a folder chosen with `GtkFileDialog::select_folder` (the backup folder, ADR 0080) and a file chosen to import or export:

- xdg-desktop-portal registers the choice in the document store with `DOCUMENT_ADD_FLAGS_PERSISTENT` (`desktop-portal/xdp-documents.c`, `full_flags`), as `read`, `write` and `grant-permissions` (`file-chooser.c`: `XDP_DOCUMENT_FLAG_WRITABLE | XDP_DOCUMENT_FLAG_DIRECTORY` unless the request says `writable: false`). `persistent` means "permanently" rather than "only for this session" (`org.freedesktop.portal.Documents.xml`). The grant is kept in the permission store under the snap's app id, `snap.rappel` (not the GApplication id), until the user removes it.
- The app sees it as `/run/user/<uid>/doc/<id>/<name>`: snapd bind-mounts `$XDG_RUNTIME_DIR/doc/by-app/snap.rappel` onto `$XDG_RUNTIME_DIR/doc` in the snap, and the `desktop` interface allows `owner /run/user/[0-9]*/doc/{,*/} r` and `/run/user/[0-9]*/doc/*/** rw` (`interfaces/builtin/desktop.go`). The document ID of a folder stays the same across restarts, so the path saved in `auto-backup.ini` stays valid.
- The backup job (`rappel_core::auto_backup::run`) opens the folder, writes a hidden partial file, renames it, and deletes its own old backups: creating, writing, renaming and unlinking inside the granted folder are `w` under that rule, and the document portal's FUSE file system allows them for a writable directory document. No lock (`k`) is needed by that path.

Not verified here, because no snap of Rappel could be installed: that the path survives a logout (the FUSE mount is recreated per session with the same IDs, by design), and that `canonicalize` of the portal path behaves as ADR 0080 expects. The maintainer's checklist covers it.

### Also found (no stop condition)

- **The application id on the session bus needs a `dbus` slot.** GApplication owns `io.github.rappelapp.Rappel` on the session bus. Under strict confinement that `RequestName` is allowed but the AppArmor `dbus (bind)` is not, unless the snap declares a slot `interface: dbus, bus: session, name: io.github.rappelapp.Rappel` (snapd's desktop interfaces only let snaps own `org.kde.StatusNotifierItem-*` and `org.freedesktop.StatusNotifierItem-*`, `desktop_legacy.go`). Without it, GApplication's registration would fail ("Failed to register", the app does not start) or Rappel would have to give up single-instance behaviour (`NON_UNIQUE`). Medium confidence: read from the policy, not run; the installed `snap-store` snap, built with the same extension, declares exactly such a slot for its own id (`/snap/snap-store/current/meta/snap.yaml`: `dbus-name: {interface: dbus, bus: session, name: io.snapcraft.Store}`). A slot is not a plug, but it is a permission outside the brief's list: QUESTIONS.md, question 2.
- The `gnome` extension adds, by itself, the content plugs `gnome-core26`, `gtk-3-themes`, `icon-themes`, `sound-themes` and the GPU content plug, and `desktop`, `desktop-legacy`, `opengl`, `wayland`, `x11` to the app. On core26 it no longer adds `gsettings` (`if base <= 24`). Rappel would add only `audio-playback`.
- The extension binds `/usr/libexec/glycin-loaders` to the platform's, so the loader binaries are there; glycin 3.1 finds their `glycin-loaders/2+/conf.d` through the XDG data folders (`config.rs`), which the launcher points at `$SNAP_DESKTOP_RUNTIME/usr/share` (`XDG_DATA_DIRS`). The loaders themselves work (Canonical's patched library uses them).

## Decision

- **Base and extension, once unblocked:** `base: core26` with the `gnome` extension (`gnome-core26` platform, libadwaita 1.10, GTK 4.24, GtkSourceView 5.22).
- **Stop.** Finding 2 hits the brief's stop condition. No `snap/snapcraft.yaml`, no install script, no CI workflow, no change to the image code in this slice. The question, with options and a recommendation, is in `docs/briefs/QUESTIONS.md` (Phase 3b, question 1).
- **Plan for the packaging step**, so that the next run does not redo this check:
  - Install layout without Meson: the binary at `usr/bin/rappel`; `.mo` files at `usr/share/locale/<lang>/LC_MESSAGES/rappel.mo` from the build script's `OUT_DIR` (it already compiles them, ADR 0059); the `.desktop` file and metainfo installed without `.in`, with the translations merged by the same code the tests use (`i18n::extract::merge_desktop`, `merge_metainfo`), written by the build script next to the `.mo` files, so no `msgfmt` is needed; the icons under `usr/share/icons/hicolor/`. One script (`scripts/snap-install.sh <destdir>`) run by snapcraft's `override-build`, and a test in the gate that runs it into a temporary directory and checks every path and name against the app id.
  - `snapcraft.yaml`: name `rappel`, `confinement: strict`, `grade` per question 4, `adopt-info` from the metainfo for title, summary, description and icon, version set from `Cargo.toml` with `craftctl set version=`, `RAPPEL_LOCALEDIR=$SNAP/usr/share/locale`, plugs `audio-playback` plus what the extension adds, no `home` and no `removable-media`; `common-id: io.github.rappelapp.Rappel` so the desktop file and metainfo keep the app id.
  - CI: `.github/workflows/snap.yml` on `v*` tags and `workflow_dispatch` only (never on pull requests, so it never slows `check` or runs on docs), `permissions: contents: read`, a maintained build action pinned by commit SHA, the `.snap` uploaded as a workflow artifact; no store token anywhere.

## Consequences

- Phase 3b cannot end with a snap until the maintainer answers question 1. Slices 8 to 10 do not depend on the snap's files, except slice 8's "the snap's description matches" the metainfo, which `adopt-info` will give for free once the snap exists.
- If the answer is to accept the snap's confinement as the image boundary, ADR 0055's row 42 and SPEC §4.3 must say so, and Rappel must select `SandboxSelector::NotSandboxed` explicitly in a snap: with `Auto`, images do not show at all (finding 2).
- The README's install section is unchanged until a snap exists.

## Sources

- snapcraft 9.2.1, `snapcraft/extensions/gnome.py` (`_SDK_SNAP`, `get_root_snippet`, `get_app_snippet`) and `share/snapcraft/extensions/desktop/common/desktop-exports`, as installed at `/snap/snapcraft/current`.
- `gnome-core26` revision 33 and `gnome-core26-sdk` revision 25 (`snap download`, 2026-11-09); the SDK's recipe, part `glycin` (`source-tag: '2.2.1'`, `patch -p1 < $CRAFT_PROJECT_DIR/patches/glycin-snap-no-sandbox.patch`). Also on GitHub: <https://github.com/ubuntu/gnome-sdk/tree/gnome-core26-sdk>, patch at `patches/glycin-snap-no-sandbox.patch`.
- glycin 3.1.0 (`~/.cargo/registry`): `src/util.rs` (`RunEnvironment::cached`), `src/sandbox.rs` (`bwrap_command`, `check_bwrap_syscalls_blocked_internal`, `INHERITED_ENVIRONMENT_VARIABLES`), `src/api_loader.rs` (`Loader::sandbox_selector`).
- snapd, branch `master` (2026-11-09): `interfaces/builtin/browser_support.go` (the only `userns` grant), `interfaces/builtin/desktop.go` (document portal rules and mount), `interfaces/builtin/desktop_legacy.go` (`dbus (bind)` rules).
- xdg-desktop-portal, branch `main` (2026-11-09): `desktop-portal/xdp-documents.c`, `desktop-portal/file-chooser.c`, `data/org.freedesktop.portal.Documents.xml`.
- `gst-inspect-1.0` on the development machine (GStreamer 1.28.7): ranks of `opusparse` (none), `faad` (secondary), `pulsesink` (primary).

## Amendment (2026-11-10): option A, the snap's confinement is the image boundary

The maintainer answered question 1 with **option A** (`docs/briefs/phase-3b-snap.md`): inside Rappel's own snap only, glycin's loader runs without bubblewrap, and the snap's strict confinement is the boundary. Built in slice 1 of that brief; the details are in ADR 0055's amendment of the same date.

- Rappel selects `SandboxSelector::NotSandboxed` explicitly, as the consequence above said, so glycin never tries bubblewrap in the snap (with `Auto`, finding 2: every image would fail). It does so only when `$SNAP` is an absolute path, `$SNAP_NAME` is `rappel` and the running binary resolves to a path inside `$SNAP` (`render/loader_sandbox.rs`, `choose`). Anywhere else, including a build run from `target/` in a snap shell or a shell with `$SNAP` set by hand, the image code behaves as before.
- glycin is not patched. Unlike Canonical's patch of the C library (finding 2, item 1), the Rust crate 3.1 sets no seccomp filter of its own for `NotSandboxed`: the loader is a plain child process, with only the snap's AppArmor and seccomp profile around it, as around Rappel.
- Size limits, the decode timeout, the SVG refusal and the four accepted formats are unchanged.
- One `info` line at start-up says which mode is in use.
- **Residual risk:** a decoder exploit in a hostile image could reach whatever Rappel can reach in the snap: Rappel's data folder (database, media, settings) and the backup folder granted through the portal, which it could read, change or delete; it still has no network and no home folder. Accepted by the maintainer.
- **Option B, for later:** a snap mode in glycin with a seccomp filter and a memory limit for the loader, proposed in `docs/notes/glycin-snap-mode.md` (written, not filed upstream).
