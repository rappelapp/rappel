# 83. The store listing for 0.4.0: new screenshots, tag-pinned URLs, release notes held back

- Status: Accepted
- Date: 2026-11-10
- Refers to: ADR 0060 (metainfo and store screenshots), ADR 0074 (icon), ADR 0075 (app id), Phase 3b brief slice 8, ADR 0082 (snap stopped)

## Decision

**Screenshots.** The store set is now seven, retaken with `scripts/screenshot.py` under Xvfb in English, light style, GNOME's window buttons and blue accent, from `fill_scratch_db -- <dir> 0 --demo --probability` (scratch data only): `review.png` (default, a calculus card with its answer), `decks.png`, `deck.png` (a deck's page: the tags said once, two group headers, "Only the shared tags" and "Also Analyse MAT3101", and rows with drawn maths on their backs), `all-cards.png`, `statistics.png` (the new month-at-a-time card), `editor.png` (Markdown with a live preview) and `phone.png` (a flag card at 360 × 640). No key hints are visible. Together about 290 KB; each is under the 300 KB limit of the test. The harness has no compositor, so there is still no window shadow (ADR 0060's "not met" stands for Flathub).

**URLs pinned to the tag.** The metainfo and the test name `…/rappel/v0.4.0/data/screenshots/…` instead of `main`, as ADR 0060 suggested for release time, so a store listing of 0.4.0 keeps showing 0.4.0's screens. The tag exists only once the maintainer tags the release; until then the URLs 404 and `appstreamcli --no-net` (the test) does not fetch them.

**Release notes held back.** The gate requires the metainfo's releases to be exactly the versions of `CHANGELOG.md`, and `Cargo.toml` still says 0.4.0-beta.1 until the version slice. A `0.4.0` entry added now would fail the gate. The text is kept in a comment in the metainfo; the slice that bumps the version turns it into the `<release version="0.4.0">` entry in the same change as the changelog's `[0.4.0]`.

**The snap.** Packaging is stopped (ADR 0082), so there is no `snapcraft.yaml`. The metainfo's summary and description are final; the snap will copy them when packaging resumes.

## Reproducing the set

Data: `fill_scratch_db -- <dir> 0 --demo --probability`. For `deck.png` and `all-cards.png` the groups are balanced in a copy, so that both group headers are above the fold: the cards with `seq` 58, 59, 62 and 66 to 69 of the "SIC 3101 – Probabilités" deck also get the tags "Analyse" and "MAT3101" (two `INSERT OR IGNORE INTO card_tags` per card, with Python's `sqlite3` on the copy; the scratch data is throwaway). The step strings of each shot are in the README's "Retake the store screenshots". `all-cards.png` moves the pointer to the header bar before the shot, so no row is highlighted.

## Release notes

The held-back 0.4.0 notes open with one sentence for people coming from 0.1.0 (the betas added formulas, media, tags, search, statistics, export and import, Undo, phones and French), then list this release's changes and the SQLite security fix. A test now checks that, once the crate has a release version, `SCREENSHOT_URL` names its tag.
