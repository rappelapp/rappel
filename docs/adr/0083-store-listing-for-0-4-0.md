# 83. The store listing for 0.4.0: new screenshots, tag-pinned URLs, release notes held back

- Status: Accepted
- Date: 2026-11-10
- Refers to: ADR 0060 (metainfo and store screenshots), ADR 0074 (icon), ADR 0075 (app id), Phase 3b brief slice 8, ADR 0082 (snap stopped)

## Decision

**Screenshots.** The store set is now seven, retaken with `scripts/screenshot.py` under Xvfb in English, light style, GNOME's window buttons and blue accent, from `fill_scratch_db -- <dir> 0 --demo --probability` (scratch data only): `review.png` (default, a calculus card with its answer), `decks.png`, `deck.png` (a deck's page: the tags said once, the group header, drawn maths on both sides), `all-cards.png`, `statistics.png` (the new month-at-a-time card), `editor.png` (Markdown with a live preview) and `phone.png` (a flag card at 360 × 640). No key hints are visible. Together about 290 KB; each is under the 300 KB limit of the test. The harness has no compositor, so there is still no window shadow (ADR 0060's "not met" stands for Flathub).

**URLs pinned to the tag.** The metainfo and the test name `…/rappel/v0.4.0/data/screenshots/…` instead of `main`, as ADR 0060 suggested for release time, so a store listing of 0.4.0 keeps showing 0.4.0's screens. The tag exists only once the maintainer tags the release; until then the URLs 404 and `appstreamcli --no-net` (the test) does not fetch them.

**Release notes held back.** The gate requires the metainfo's releases to be exactly the versions of `CHANGELOG.md`, and `Cargo.toml` still says 0.4.0-beta.1 until the version slice. A `0.4.0` entry added now would fail the gate. The text is kept in a comment in the metainfo; the slice that bumps the version turns it into the `<release version="0.4.0">` entry in the same change as the changelog's `[0.4.0]`.

**The snap.** Packaging is stopped (ADR 0082), so there is no `snapcraft.yaml`. The metainfo's summary and description are final; the snap will copy them when packaging resumes.
