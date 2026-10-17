# 38. The roadmap in phases

- Status: Accepted
- Date: 2026-10-17
- Amends: ADR 0034 (the order of work; §4.10 becomes the internal code clean-up), SPEC title and status line, §4 (the roadmap, §4.8, §4.10, new §4.11), §5
- Refers to: ADR 0025 (hardening, round 1)

## Context

ADR 0034 set one flat order from the browser to the release. It left no point where the app is finished enough to ship before media and mobile work start, and it read "clean-up" as a user feature (removing unused media files), while it was meant as the internal code clean-up slice.

## Decision

**Three phases, each ending in a release.**
- *Phase 1, text only, ends in v0.2.0.* Done: browser and search, suspend and delete during review. Then: the missing features (keyboard shortcuts dialog, About dialog with the font credits, remembering the window size and the last deck); a UI overhaul (a visual pass on every screen, the accessibility issues); code clean-up and refactor; hardening, round 2; v0.2.0-beta, then v0.2.0.
- *Phase 2, ends in v0.3.0.* Streaming import, images, audio, removing unused media files, phone and touch adaptivity, Anki `.apkg` import, then a refactor and hardening.
- *Phase 3, ends in v1.0.* Translation, a testing campaign, packaging, release.

*Why.* A text-only app that is complete, consistent and hardened can be released and used before media and mobile grow the surface. Each phase ends with a refactor and hardening so the next one starts from clean code. Streaming import still precedes images and audio because they make files large; Anki import follows them so `<img>` and `[sound:]` have somewhere to go. Translation stays after the UI settles, and a testing campaign comes before 1.0 because 1.0 fixes the export format's compatibility promise (§4.1).

**§4.10 is the code clean-up and refactor**: internal, no user-visible change. The user-facing removal of unused media files becomes its own phase 2 item, after audio; its old rules are dropped, and it needs its own design decision, because undo, restore and backups must never lose a file.

**Nested decks** move from "not in v0.2" to "Not scheduled".

## Consequences

- The SPEC's §4 lists the phases; Anki import moves from §5 into §4 as §4.11, and §5 holds phase 3 and the unscheduled items.
- Until that slice lands, files that no card uses stay in the media store once images and audio exist. The slice starts with its own ADR.
- No code changes in this slice.
