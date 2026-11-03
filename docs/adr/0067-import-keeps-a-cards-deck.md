# 67. An import keeps a card's own deck

- Status: Accepted
- Date: 2026-11-02
- Refers to: SPEC §4.1; ADR 0035 (`suspended` kept by an import), ADR 0064 (Move to Deck…), ADR 0066 (Merge Into…); amends the merge rules of ADR 0017 and SPEC §4.1

## Context

An import compares each card of the file with the database's card of the same id. Until now any difference other than `deleted_at` (and, since ADR 0035, `suspended`) was a conflict, and one conflict refuses the whole file. A card moved to another deck (Move to Deck…, All Cards' Move…, or Merge Into…, which moves every card of a deck) differs from an older export only in `deck_id`. So an export made before any move could never be imported again, and people use exports as backups.

## Decision

**A card that differs only in its deck (and in `suspended` or `deleted_at`, as before) is already present, and the database keeps its own deck.** An import never reorganises cards, as it never changes how a live card is studied (ADR 0035). Not a format change and not a migration: the rule is in `classify_card` (`interchange/engine.rs`).

- A card that differs in its deck **and** anything else (front, back, times) is still a conflict.
- The file's deck for the card is added or revived like any deck, by its own row. A deck that is new, or deleted here and live in the file, comes back without the card, which stays where the database has it. In particular a deck merged away (ADR 0066) comes back empty when an older export is imported, as a deleted deck always is.
- A card deleted here and live in the file is revived (SPEC §4.1) **in its own deck** when that deck is live after the import: live here, or revived by this import (the decks are read before the cards). Otherwise it stays deleted: reviving it would make a live card in a deleted deck, and the import does not bring that deck back. Cards a revived deck brings back with it follow their own row, as before: the file's row decides whether the card is revived, and the rule above where it goes.
- The plan's summary counts such a card as present (or revived), never as a conflict, and counts apart the ones that stay live in another deck than the file's (`ImportSummary::cards_kept_in_their_deck`). The import dialog says so: "3 cards are in another deck in the file and stay in the ones they are in now.", so a reorganisation made elsewhere is not dropped silently. Importing the same file twice still changes nothing, and export → import → export still gives the same bytes.

## Consequences

- An export made before a merge or a move imports again: everything new in it is added, the moved cards stay where they are.
- **Two machines.** Cards moved on machine A keep B's deck when A's export is imported on B: if A moved twenty cards into a new deck "Verbs", "Verbs" arrives on B empty, and the dialog says that twenty cards stay where they are. Moving them there on B is the user's choice.
- **After a merge, an older backup** brings the merged deck back empty: it is live in that file, and an import revives a deleted deck (SPEC §4.1); its cards stay in the deck they were merged into.
- The differential test's model of the import (`interchange/tests/oracle.rs`) has the rule too, written apart from the engine, and the random histories of the property tests move cards and merge decks, so files exported before a reorganisation are checked against it; a property test imports such a file after random moves, merges, deletes and undos (no conflict but a deck name taken since, every live card stays in its deck, no live card in a deleted deck, the count, a second import changing nothing).
- The cost is that an import can no longer be used to put cards back into the deck an export had them in. That was never offered: it was refused instead.
