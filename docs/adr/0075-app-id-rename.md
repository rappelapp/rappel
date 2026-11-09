# 75. The app ID becomes io.github.rappelapp.Rappel (#130)

- Status: Accepted
- Date: 2026-11-09
- Refers to: issue #130; ADR 0020 (rename to Rappel, which fixed the old ID), ADR 0060 (desktop integration drafts), ADR 0074 (designed app icon)

## Context

The repository moved from the maintainer's personal account to the `rappelapp` organisation (5 November). The app ID `io.github.rsmirke.Rappel` was derived from the old account, and the files in `data/`, the About dialog and the metainfo named the maintainer personally. Nothing has been installed or published under the old ID (the files in `data/` are drafts, ADR 0060), so changing it now costs the least it ever will; after a Snap or Flatpak release it would be much harder.

## Decision

**Changed**
- The application ID is `io.github.rappelapp.Rappel` (`rappel_core::APP_ID`). The four files in `data/` (desktop entry, metainfo, scalable and symbolic icons) were renamed with `git mv` to match, as were the GResource prefix `/io/github/rappelapp/Rappel/`, every `#[template(resource = ...)]`, `po/POTFILES.in` and the source references in the `.pot`/`.po` files.
- Every URL (workspace `repository`, About, metainfo homepage, bug tracker, VCS, screenshots, the translation bug address, the CHANGELOG compare links) points to `https://github.com/rappelapp/rappel`.
- The developer is "The Rappel Project" in About (developer, copyright) and in the metainfo, with developer id `io.github.rappelapp`. The personal name is gone from the code, the metainfo and the icon's comment.
- `CLAUDE.md`, `SPEC.md`, `README.md` and `ARCHITECTURE.md` use the new ID and URL. The tests that check the desktop entry and the metainfo against the code derive the file names from `APP_ID`, so they follow without being relaxed.

**Not changed**
- The data directory stays `~/.local/share/rappel/` (`rappel-dev` in debug builds, `RAPPEL_DATA_DIR` overrides). It is named after the app, not the ID, so no user data moves.
- Historical ADRs, past CHANGELOG entries and old briefs keep the old ID as they were written. `translator-credits` is untouched.

## Consequences

- Nothing stored is keyed by the app ID. Rappel uses no `GSettings`/dconf (the window state is a key file in the data directory, `window_state.rs`), no keyring, and no portal permission store entry of its own, so there is nothing to migrate. If a user had granted a portal permission to the app while running it from source, the portal may ask again (GTK's application ID is also the D-Bus name).
- The ID is the D-Bus name and the single-instance key: an old build and a new build are different applications and can run side by side.
- A desktop entry or icons installed by hand under the old ID stay behind; the old ones are not removed by anything and should be deleted by hand (the commands are in the slice report).
- Packaging (Snap, Flatpak) starts from the new ID, so it never has to carry a rename.
