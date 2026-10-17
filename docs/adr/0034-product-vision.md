# 34. Product vision and the order of work

- Status: Accepted
- Date: 2026-10-17
- Amends: SPEC status line, the new Product vision section, §1 (principle 4), §4 (order, §4.1 streaming import, new §4.9 and §4.10), §5, §6
- Amended by: ADR 0038 (the roadmap in phases; §4.10 is the internal code clean-up)
- Refers to: ADR 0017 (the export format), ADR 0029 (tags are never deleted, so clean-up leaves them)

## Context

The SPEC said what to build but not what kind of app Rappel is. Without that, each feature is argued from zero, and Anki, the obvious reference, pulls toward note types, add-ons and presets that a small GNOME app should not carry.

## Decision

**Vision.** Rappel is an opinionated, modern GNOME flashcard app, closer to Mochi than to Anki. Efficiency is not maximalism. Smart behaviour is built in and invisible until needed (progressive disclosure). Every setting must justify itself. Anki is a source to import from, not a model: no note types, add-ons, field templates or per-deck presets. Card links belong to the separate method app. The text is at the top of the SPEC.

**Order.** Browser and search, streaming import, images, audio, clean-up, polish (phone and touch), Anki import, translation, release.
- *Streaming import* is new work on §4.1: reading the archive entry by entry so memory stays bounded once images and audio make files large. Format and merge rules do not change.
- *Audio* and *clean-up* are new (§4.9, §4.10). Audio reuses the media store and its `media:` rule; clean-up removes unreferenced media and unused tags only on request.
- *Anki import* stays, one way, and drops what Rappel has no place for.
- *Release* (Meson, Flatpak, AppStream) moves from "Later" into the order, last.

*Why this order.* The browser comes first because search and bulk actions are what a growing collection lacks most, and suspend and delete during review come with it. Streaming import precedes images and audio because those make files large. Polish for phone and touch comes after the content features so it audits the finished screens. Anki import comes before translation because it brings users in, while translating a moving UI is wasted work; release is last.

**Removed.** Cloze cards. They need a card type and a template, which the vision rules out.

**Added to Later.** Reversed cards, as an option on one card, not a note type.

## Consequences

- A request for a note type, a template or a per-deck preset is declined by reference to this ADR.
- Audio and streaming import need their own ADRs when built (formats and limits, memory bound).
- No code changes in this slice.
