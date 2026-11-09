# Phase 3a — brief for batch mode (target: 0.4.0-beta.1)

Approved by the maintainer on 2026-11-02 (decisions D1–D10 of the Phase 3a
proposal: all as recommended).

Same standards as Phases 1 and 2: GNOME HIG, libadwaita docs checked, every
slice gated, reviewed (reviewer agent, Opus, until nothing blocking),
screenshotted (light, dark, 1000 px, 360×640 portrait, 640×360 landscape, and
French at 360×640 for any new string). Integration branch: `phase-3a`, created
from `main` at `v0.3.0-beta.1` or later. Model: Opus for planning,
implementation and review.

Pre-approved (not stop conditions):
- Migration 0012: a nullable `archived_at` on `decks`, with the usual test
  migrating a DB made by 0.3.0-beta.1.
- Export format v2 gains an optional `archived_at` on deck records
  (written always, a file without it reads as not archived; the format number
  stays 2, as for `suspended`, ADR 0035).
- Merging decks soft-deletes the emptied deck (`deleted_at`, undoable), as
  Delete Deck already does. No other deletion of user data.
- New dependencies whose licence is already allowed by `deny.toml` (none is
  expected).
Any other migration, format change, licence or deletion is a stop condition.

## Slices, in order

*Note for the record (added in Phase 3b slice 0): #117 (the "Ctrl ⏎ to save" hint) was folded into slice 7 (#121), and slice 7b (#119, the duplicate-front note in the card editor) was added during the run (#122). Neither is listed below.*
0. Housekeeping (docs only). SPEC: status line and §4/§5 roadmap — Phase 3a
   (this brief) ends in 0.4.0, Phase 3b (publishing: Snap, self-hosted Flatpak,
   finished metainfo, icon) ends in 1.0; 0.2.0 and 0.3.0 are not released as
   stable; §4.5, §4.6 gain the new features as "to do". CLAUDE.md:
   the status line, and batch mode's names (`docs/briefs/phase-3a.md`,
   `phase-3a`), nothing else. QUESTIONS.md: Phase 2 question 1 answered
   (glycin advisories kept, review by 2027-01-16). CHANGELOG "Unreleased".
1. Small polish:
   - #105: the Space / 1 / 2 key hints leave the review page; the grade and
     reveal buttons say their key in their tooltip ("Pass (2)"), the
     accessible description keeps it, the shortcuts dialog is unchanged.
   - #106: Ctrl+A selects all cards on All Cards (and on the deck page after
     slice 2), entering selection mode if needed; listed in the shortcuts
     dialog. Not active while a text field has focus (it selects the text).
   - #102: "Move to Deck…" in each card's ⋮ menu on the deck page (above
     Delete) and in the review page's Card Menu, with an Undo toast; reuses
     All Cards' deck chooser and `move_cards` / `unmove_cards`. During review
     it leaves the session like Suspend does.
   - Redo PR #32 on top of `main` (a table's border hugs its columns,
     `halign: Start` on the grid; ADR 0019 amended), then note that #32 can
     be closed.
2. Selection mode on the deck page (#104): a "Select Cards" button, the same
   selection mode, action bar and Undo as All Cards (move, tags, suspend,
   reset, delete). First extract All Cards' selection mode into one shared
   component in `crates/app/src/ui/` (or `widgets/`), so the two pages cannot
   drift; All Cards' behaviour and tests stay unchanged.
3. Merge decks (#103): "Merge Into…" in each deck's ⋮ menu (deck list
   and deck page). A dialog picks the target deck (not itself, not archived),
   says how many cards move, and confirms with a non-destructive default.
   Core: one transaction moves every card (live and deleted rows keep their
   history) and soft-deletes the source deck; one Undo restores both exactly.
   Property test: merge then undo is the identity on the database. No deck
   list multi-select.
4. Archive a deck: "Archive" in the deck's ⋮ menu (Undo toast). An
   archived deck leaves the deck list, the study counts, the global
   statistics' forecast and retention, and the window-state "last deck"; its
   cards stay in All Cards (with a filter "Archived decks: shown / hidden",
   hidden by default) and in exports. An "Archived Decks" row at the end of
   the deck list (only when one exists) opens a page listing them with
   Unarchive. Imports keep the flag. Migration 0012 and the export field as
   pre-approved; ADR.
5. Study by tag: "Study Cards Tagged …" from a tag: on the deck page's
   tags and from All Cards' tag filter. A normal session (same FSRS, same
   daily limits shared with the decks, written to the log) over due and new
   cards carrying the tag in any non-archived deck. The session's title says
   the tag. Core: the session builder takes a scope (deck or tag); pure and
   tested. No saved filters, no custom study screens.
6. Practice mode: "Practice" for a deck or a tag, offered on the
   "all done for today" status page and in the deck's ⋮ menu. It shows cards
   in an order of the scheduler's choosing (lowest retrievability first),
   with the same Fail / Pass buttons; **nothing is written** (no review
   event, no change to any card), Undo just steps back, the summary says
   "Practice: nothing was rescheduled". A test asserts the database is
   byte-identical before and after a practice session. ADR explaining why
   (FSRS would learn from a recall at t ≈ 0).
7. Phase 2 leftovers:
   - French: All Cards' filter drop-downs no longer cut their labels; header
     subtitles at 360 px with minimise and maximise shown.
   - Shrinking the window with Add Tags open must not turn the typed text
     into a pill (a test with the harness).
   - Refusing an over-large import file: decided early from its size where
     possible, and the worker gets a Cancel.
   - Exports whose media add up to more than 2 GiB: before writing,
     the export says the file could not be imported again and offers to
     export without media or cancel; SPEC §4.1 and ADR 0054 amended. No
     format change.
8. Code clean-up #4 and hardening round 4 (SPEC §4.10): dead code and
   duplication from slices 1–7, ARCHITECTURE.md brought up to date; attacks on
   the new paths (an import with `archived_at` of every odd type, a merge
   across thousands of cards with undo, a practice session interrupted by a
   crash, Ctrl+A on 10,000 cards); the ADR 0063 table extended. No migration,
   no format change.
9. Version 0.4.0-beta.1 (Cargo.toml, Cargo.lock, CHANGELOG, metainfo as
   `0.4.0~beta.1`; do NOT tag); final report `docs/briefs/phase-3a-report.md`
   with decisions, questions, screenshot paths and one checklist for the test
   week (including #48, the Orca listening test, with exact steps); draft PR
   `phase-3a` → `main`.
