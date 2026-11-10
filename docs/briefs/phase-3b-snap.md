# Phase 3b, part 2: the snap (brief for batch mode, target 0.4.1 on the Snap Store beta channel)

*Lead engineer, 10 November 2026. Commit it as `docs/briefs/phase-3b-snap.md` in slice 0.*

## In short
v0.4.0 went out as a source release because the image sandbox can't run inside a snap (ADR 0082). On 10 November the maintainer chose **option A**: inside Rappel's own snap only, run the image loader without bubblewrap. The snap's strict confinement is the boundary (no network, no home folder). The size limits, timeout and SVG refusal stay. This short batch builds that, packages Rappel as ADR 0082 planned, and ends with **0.4.1**. CI builds the `.snap`, and the maintainer uploads it to **beta**.

## Setup
- Integration branch `phase-3b-snap`, created from `main` after `v0.4.0` is tagged. It matches the `phase-*` ruleset.
- The same standards and gate as Phase 3b, and the same model table:
  - the orchestrator runs on **Sonnet**;
  - slices 1 and 2 run on **Opus**, slices 0 and 3 on **Sonnet**;
  - the reviewer always runs on **Opus**;
  - a Sonnet slice that is still blocked after 2 review rounds moves to Opus.
- Answers to QUESTIONS.md, Phase 3b. Q1 is **A** (maintainer, 10 Nov). Q2 to Q5 follow the recommendations: the `dbus` slot is allowed, data is kept per revision with a README note, the grade is `stable`, and the `.partial` clean-up stays. If the maintainer answers otherwise before the batch starts, they edit this line.

### Pre-approved
- **The image loader runs unsandboxed in the snap only,** under the conditions in slice 1.
- `snap/snapcraft.yaml`, `scripts/snap-install.sh` and `.github/workflows/snap.yml`, as planned in ADR 0082.
- Plugs: the desktop set and `audio-playback`, plus the `dbus` slot for `io.github.rappelapp.Rappel`.
- Staging `faad` for M4A, if its licence is allowed (check `deny.toml` and the snap's licence field). If it is not, M4A is not supported in the snap, and the README says so.

### Stop conditions
- Any change to how images are loaded **outside** the snap.
- A `home`, `removable-media`, `network` or `browser-support` plug, or classic confinement.
- Any migration, export-format change, or new deletion of user data.
- A store token or secret in the repo or CI.

## Slices
**0. Housekeeping** (Sonnet, docs).
- Commit this brief.
- CLAUDE.md batch names: `docs/briefs/phase-3b-snap.md` and `phase-3b-snap`.
- QUESTIONS.md: record the answers above.
- CHANGELOG "Unreleased".

**1. Images in the snap** (Opus).
- A pure, tested function chooses the loader's sandbox. It picks "not sandboxed" **only** when all of these hold: `$SNAP` is set, `$SNAP_NAME` is `rappel`, and the binary runs from under `$SNAP`. In every other case, current behaviour stays (ADR 0055): bubblewrap where it works, and the existing refusal and warning otherwise.
- Use glycin's own selector for "not sandboxed" (read the crate source in `~/.cargo/registry`; do not patch glycin).
- Everything else stays the same: size limits, decode timeout, SVG refusal, the decoder allow-list.
- Log one line at start-up saying which mode is in use.
- Amend ADR 0055 and ADR 0082 with the decision and its residual risk: a decoder exploit in a hostile image could reach Rappel's data and the backup folder.
- Write a short `docs/notes/glycin-snap-mode.md`, an upstream proposal for a snap mode with seccomp and a memory limit (option B, for later). Write it only; do not file it.

**2. Packaging** (Opus). As in ADR 0082:
- `core26` with the `gnome` extension;
- `scripts/snap-install.sh` installs the binary, `.mo` files, desktop file, metainfo and icons, and has a test;
- `RAPPEL_LOCALEDIR` set;
- strict confinement, `grade: stable`;
- title, summary, description and icon from the metainfo.

`.github/workflows/snap.yml` builds the snap with `snapcore/action-build`:
- on `v*` tags and manual dispatch, uploading the `.snap` as a workflow artifact;
- also on PRs that touch `snap/**`, `scripts/snap-install.sh` or the workflow itself, so this slice's PR proves that it builds;
- never on docs-only PRs, and never publishing anything.

A README section covers installing the snap, where the data lives, copying data from the native build, and the revert note (data per revision).

**3. Version 0.4.1** (Sonnet).
- Bump the Cargo files, CHANGELOG and metainfo. Do not tag.
- A short report, `docs/briefs/phase-3b-snap-report.md`, containing:
  - what changed;
  - the snap checklist from the Phase 3b report (the section "The snap: none to install yet"), updated;
  - how to download the CI artifact (`gh run download`).
- Draft PR `phase-3b-snap` → `main`.

## After the batch (the maintainer)
1. Merge, tag `v0.4.1`, create the release. The snap workflow builds the `.snap` from the tag.
2. `gh run download`, then `sudo snap install --dangerous ./rappel_0.4.1_amd64.snap`. Copy the data and run the snap checklist.
3. `snapcraft upload --release=beta rappel_0.4.1_amd64.snap`.
