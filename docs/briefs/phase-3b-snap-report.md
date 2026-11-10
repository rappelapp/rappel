# Phase 3b, part 2 (the snap): report

*10 November 2026. Brief: `docs/briefs/phase-3b-snap.md`. Integration branch `phase-3b-snap`, draft PR `phase-3b-snap` → `main`.*

## What changed
Rappel can be built as a strictly confined snap, version **0.4.1** (not tagged). The app itself is the same as 0.4.0.

| Slice | PR | ADR | Model | Review rounds | What |
|---|---|---|---|---|---|
| 0. Housekeeping | #145 | — | Sonnet | 1 | The brief committed, CLAUDE.md batch names, your answers in QUESTIONS.md, CHANGELOG. |
| 1. Images in the snap | #146 | 0055, 0082 (amended) | Opus | 1 | Inside Rappel's own snap only, glycin's image loader runs without bubblewrap (`SandboxSelector::NotSandboxed`). Chosen by a pure, tested function (`render/loader_sandbox.rs`): `$SNAP` set and absolute, `$SNAP_NAME` is `rappel`, and the running binary is inside `$SNAP`; everything else keeps glycin's own choice, as before. Sizes, timeout and SVG refusal are untouched. One start-up log line says which mode is in use. `docs/notes/glycin-snap-mode.md` is the upstream proposal for option B (written, not filed). |
| 2. Packaging | #147 | 0085 | Opus | 1 | `snap/snapcraft.yaml` (core26, `gnome` extension, strict, `grade: stable`, the desktop plugs, `audio-playback`, the `dbus` slot), `scripts/snap-install.sh` with a test in the gate, the `.mo` files and merged desktop/metainfo files made by `build.rs`, `.github/workflows/snap.yml`, a README section. |
| 3. Version 0.4.1 | this PR | — | Sonnet | — | Cargo files, CHANGELOG, metainfo, status lines in CLAUDE.md and SPEC.md, this report. |

No stop condition was hit: no migration, no export-format change, no new Cargo dependency, no forbidden plug, no secret in the repo or CI. All reviews were by Opus with no blocking finding. The one reviewer point I acted on before merging #147 was the README's `cp -a` recipe, which could mix the snap's fresh collection with the copied database; it now moves the fresh folder aside first.

## Things to know
- **M4A does not play in the snap.** The AAC decoder (`libfaad2`) is GPL-2.0-or-later, which is not on `deny.toml`'s allow-list, so by the brief's rule it was not staged. QUESTIONS.md, "Phase 3b, part 2", question 1 gives option B (allow the licence and stage `faad`). I recommend leaving it for now.
- **The image loader in the snap has no seccomp filter and no memory limit** (glycin only applies those with bubblewrap). The boundary is the snap's confinement: no network, no home folder. A decoder exploit in a hostile image could still reach Rappel's own data and the backup folder (ADR 0055 amendment).
- **The rule checks the snap's environment, not its confinement.** A snap installed with `--devmode` would run the loader with no real boundary. Documented in ADR 0055 and 0082; not coded, because it cannot be tested without a real snap.
- **The start-up line saying the image-loader mode appears only with `RUST_LOG=rappel=info`** (the default level is warnings).
- **The snap was built only in CI**, never on this machine (no LXD, and `--destructive-mode` refuses a 26.10 host for `core26`). CI built `rappel_0.4.0_amd64.snap` on #147. Nothing was installed or run: the `dbus` slot, the portal folder's persistence, images in the snap, French and audio are read from sources only.
- **CI warnings, not fixed:** snapcraft's linter says the `contact` field is missing (adding `contact: https://github.com/rappelapp/rappel/issues` would silence it), and `snapcore/action-build` v1.3.0 runs on Node 20, which GitHub forces onto Node 24.
- The snap workflow runs on packaging PRs only, not on every code PR (this revises ADR 0082's "never on PRs", as the brief asks).
- Store screenshot URLs in the metainfo are pinned to the `v0.4.1` tag, as the gate requires for a released version (ADR 0083). The tag does not exist until you make it, so the store images resolve only after that; they are the same files as in 0.4.0.

## Get the `.snap` from CI
After you merge, tag `v0.4.1` and create the release. The `v*` tag starts the Snap workflow, which uploads the `.snap` as a workflow artifact and never publishes.
```sh
gh run list --workflow snap.yml --limit 5          # find the run for the v0.4.1 tag
gh run download <run id> --name rappel_0.4.1_amd64.snap --dir ~/Downloads/rappel-snap
sudo snap install --dangerous ~/Downloads/rappel-snap/rappel_0.4.1_amd64.snap
```
Then, once the checklist below is done: `snapcraft upload --release=beta ~/Downloads/rappel-snap/rappel_0.4.1_amd64.snap`.

## The snap checklist (updated from the Phase 3b report)
1. Rappel starts from the app grid with its icon and name.
2. `snap connections rappel` lists only the desktop set, `audio-playback`, the content plugs the `gnome` extension adds, and the `dbus` slot. No `home`, `removable-media`, `network` or `browser-support`.
3. Images show in a card, and an SVG is still refused. `RUST_LOG=rappel=info snap run rappel` logs "image loader: not sandboxed by bubblewrap; inside the rappel snap, …".
4. MP3, Ogg and WAV play. An M4A says "could not be played" (expected).
5. A second `snap run rappel` raises the same window (the `dbus` slot).
6. `LANGUAGE=fr snap run rappel` is in French.
7. Export… and Import… go through the portal. Choose a backup folder, close, log out and back in, edit, close: the backup is written to the same folder (the portal's persistence, ADR 0080).
8. The data is in `~/snap/rappel/current/.local/share/rappel`. After a `snap refresh` the collection is there. Read the README's revert note (data per revision).
9. Light, dark, 360 px wide, keyboard only and Orca labels, once in the snap.

**Copy your data into the snap** (the snap cannot read `~/.local/share/rappel`). Export… in the native app and Import… in the snap is the safer way. Or, with both closed:
```sh
snap run rappel        # once, then close the window, so the folder exists
mv ~/snap/rappel/current/.local/share/rappel ~/snap/rappel/current/.local/share/rappel.empty
cp -a ~/.local/share/rappel ~/snap/rappel/current/.local/share/
```
The native original is not touched. `sudo snap remove --purge rappel` deletes the snap's own folder only.

## For you to decide
1. M4A in the snap: leave it unsupported (done), or allow GPL-2.0-or-later and stage `faad` (QUESTIONS.md, question 1)?
2. Add `contact:` to `snapcraft.yaml`? A one-line change.
3. Option B (a snap mode in glycin): `docs/notes/glycin-snap-mode.md` is ready to file upstream when you want.
