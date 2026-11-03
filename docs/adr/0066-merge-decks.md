# 66. Merge a deck into another, and its Undo

- Status: Accepted
- Date: 2026-11-02
- Refers to: SPEC §4.6, §4.1; ADR 0067 (an import keeps a card's own deck); `docs/briefs/phase-3a.md` (slice 3, #103); ADR 0008 and 0012 (soft delete and its undo), ADR 0013 (deck page), ADR 0035 (bulk writes), ADR 0057 (alerts' default focus), ADR 0058 (toasts at 360 px), ADR 0064 (Move to Deck…)

## Context

Users who split a subject into several decks want to put them back together (#103): "Merge Into…" in a deck's ⋮ menu, on the deck list and on the deck page. The brief asks for one transaction that moves every card, live and deleted, keeping its history, and soft-deletes the emptied deck, as Delete Deck does; one Undo that restores both exactly; and a property test that merge then undo is the identity on the database. It pre-approves the soft delete of the emptied deck. There is no migration and no export-format change.

Two existing rules meet here. Delete Deck stamps the deck and its live cards with one `deleted_at`, later than any card of the deck deleted before, and its undo revives exactly the cards with that stamp (ADR 0012). An import revives a deck deleted in the database but live in the file, with the cards deleted with it (SPEC §4.1). Moving deleted cards between decks must not let either rule revive a card the user deleted on its own.

## Decision

**Core: `Store::merge_deck(source, target) -> Merged` and `Store::unmerge_deck(&Merged) -> Deck`** (`storage/merge.rs`), each one immediate transaction that writes nothing on error.

Merge:
- Refuses the same deck (`Error::MergeIntoItself`, new) and a deck that is not live (`DeckNotFound`), before any write.
- Every card of the source, live and deleted, gets `deck_id = target` in one statement (`UPDATE cards SET deck_id = ? WHERE deck_id = ?`, on the deck's index). Nothing else of a card changes: content, `updated_at` (as in `move_cards`, a move does not change what a card says), tags, suspension, `deleted_at`, its log and so `card_schedule`. The search index has no deck column and is untouched.
- The source gets `deleted_at` exactly as Delete Deck would stamp it (`deletion_stamp`, now shared): the time, or one past the latest stamp of its cards deleted earlier. Its cards are already gone, so no card shares the stamp.
- `Merged` holds the source, the target, the stamp, the ids of the cards moved (live and deleted, oldest first) and how many were live.

**Why the delete and revive rules stay coherent.** A deleted card that moved keeps its own stamp. If the target is later deleted, Delete Deck stamps it past the latest stamp of the target's cards, which now include the moved ones, so the undo of that delete revives only the target's batch, never a card deleted before the merge (tested, also with a clock that went backwards). Reviving the merged deck (`undelete_deck`, or an import of a file where it is live) revives the cards whose deck is the source and whose stamp is the deck's: there are none, so the deck comes back empty. The decks are only ever told apart by `deck_id`, so no rule needed to change.

Unmerge, the Undo:
- The source becomes live again (`deleted_at` back to NULL) if it still has the merge's stamp. If a live deck took its name meanwhile, `DuplicateDeckName` and nothing is written, as for `undelete_deck`. A source already live again (revived by an import) stays live and gets its cards back. A source deleted again since, with another stamp (revived, then deleted by the user), is `DeckNotFound`: the later delete is the user's choice and is not undone by an older Undo.
- The target must be live; otherwise `DeckNotFound(target)` and nothing is written. Moving its cards out of a deleted target would leave them deleted with the target's stamp in a live source, and the target's own Undo would no longer find them. The user undoes the target's delete first.
- **Every recorded card that is still in the target goes back**, in one statement (the ids as a JSON array bound to `json_each`). That covers the normal case exactly, and it is what keeps merge then undo the identity. For a card changed since the merge:
  - edited or studied: it goes back with its edit and its new reviews. Undo is about where the cards are, not what they say; nothing is lost.
  - deleted: it goes back, still deleted. Leaving it in the target would split the source's history across two decks; moved back, its own "Card deleted" Undo still brings it back, now into the live source.
  - moved to a third deck: it stays there, as with `unmove_cards`: the later move was the user's choice.
- The target's own cards are never touched (they are not in the list).

**Tests** (`storage/merge/tests.rs`, `interchange/tests/merged.rs`, `storage/tests/perf.rs`): what moves and what stays, every refusal writing nothing, the stamp, a failure at the last write (a temporary trigger) leaving the database as it was, Delete Deck + Undo of the target after the merge, reviving the merged deck, study counts and statistics after the merge being the sums, each case of the Undo above, and ids that need escaping in JSON. A full-database dump helper (`test_support::dump`: every row of every table in `sqlite_master`, the search index's own tables and `card_schedule` included, plus the schema version) compares databases; the property test builds random databases (four decks, cards with reviews, resets, undos, tags, suspensions, moves and deletions, and maybe a deck deleted earlier), merges a random pair of live decks and undoes it, and asserts the dump is unchanged, and that in between no card is left in the source, the merge recorded exactly the source's cards, and the target's counts are the sums; the merge's clock may be up to a day behind the last write. The dump has each row's `rowid` (for rowid tables) and the schema (`sqlite_master`). Export after a merge round-trips byte for byte, and an export after the Undo equals the one before the merge. A merge of 10,000 cards and its Undo take about 50 ms each in a debug build (bound: 500 ms).

**An import of a file made before the merge imports as already present** (ADR 0067): a card that differs only in its deck keeps the database's deck, so the moved cards stay in the target, and the merged deck, live in that file, is revived empty, as any deleted deck would be. An import never undoes a merge by itself.

**UI** (`dialogs/merge_deck.rs`, layout `data/merge_deck_dialog.blp`):
- "Merge Into…" in the deck row's menu between Rename… and Delete (destructive last, HIG), and in the deck page's Deck Menu in a section of its own under Statistics (that menu has no Rename or Delete). It is disabled when there is no deck to merge into; `merge_deck::targets` is the one function that lists the targets (every deck but the source), so phase 3a's archive slice leaves archived decks out there for both menus.
- An `AdwAlertDialog`, "Merge Deck?", with a drop-down of the targets (searchable, a long name ends in "…" on its button: `ui::dropdown::shrinkable`) and the body "Move 12 cards from “Spanish” into “Languages” and remove “Spanish”?", which follows the drop-down. **The count is the source's live cards, suspended ones included** (`Store::card_count`): what the user sees in the deck. Deleted cards move too, but the user no longer sees them; counting them would make "12" disagree with the 10 cards on screen for no benefit. A deck with no cards says "“Spanish” has no cards to move into “Languages”. Remove “Spanish”?".
- **Cancel is the default and close response, and the drop-down has the focus** once the dialog shows, at every window size. ADR 0057 found that in a small window libadwaita focuses an alert's last response; `ui::dialog::focus_when_shown` sets the focus when the dialog maps and again from a high-priority idle, as `focus_default_when_shown` does (the two now share that code). Enter on the drop-down opens its list, so Enter never merges by accident. Merge is a suggested response, not a destructive one: Undo reverses it.
- After merging from the deck list, the source's row leaves and the focus goes to the row now at its place, else the last one (`ui::focus::index_after_removal`), set again once the dialog has closed (it gives the focus back to the menu button that opened it, now gone).
- **After merging from the deck page, the page goes back to the deck list with the focus on the target's row**, rather than replacing itself with the target's page. The source's page has nothing left to show; the list shows both effects at once (the source gone, the target's new counts) and is where the Undo's result shows too; the target is one Enter away. Pushing the target's page would also mean a page the user never opened with Back leading to the list.
- The toast is "Decks merged" (short, so it fits beside Undo at 360 px, ADR 0058), always from the deck list's page, which lives as long as the window. Its Undo works from any page: every open deck page and All Cards read their cards again, and the deck list shows the source again, focused if the list is showing (`RappelWindow::deck_unmerged`). A failed Undo says why in words (`merge_deck::not_restored_text`): "“Spanish” could not be restored: “Languages” was deleted since.", "… it was deleted again since.", "… another deck has its name now."; anything else is Delete Deck's message with the error (`ui::text::not_restored`, shared).
- No multi-select on the deck list.

## Consequences

- No migration, no export-format change. The merged deck stays in the database and in exports as a deleted deck, like any deleted deck.
- A deck that went into a merge can be found again only through the toast's Undo. A later Delete Deck of the target and its Undo work as before.
- The window's remembered "last deck" pointing at a merged deck falls back to the deck list at the next start, as for a deleted deck (`DecksPage::open` ignores `DeckNotFound`; tested).
- The Undo reloads the deck list, every open deck page and All Cards, but not an open Review or Statistics page of the target: a session there keeps the cards it started with, and its answers stay correct since they are written by card id, and Statistics shows the merged numbers until it is opened again. Only cosmetic.
