# 88. Practice This Card

- Status: Accepted
- Date: 2026-11-16
- Refers to: ADR 0070 (practice writes nothing), ADR 0069 (study by tag); the 0.4.2 request "Practice This Card in each card's menu"

## Context

Practice (ADR 0070) goes over a deck's or a tag's cards outside the schedule and writes nothing. Users also want to go over one card, the one they are looking at: from its row on a deck's page or in All Cards, or from the Card Menu while studying, without touching its schedule.

## Decision

- **One entry, "Practice This Card"**, in a card row's menu (deck page and All Cards, after Card Information) and in the review page's Card Menu (after Card Information). It is disabled for a suspended card and for one of an archived deck (`card_row::text::practice_card_offered`, the one-card case of `pages::practice_offered`); a deleted card has no row. In the review page it is shown only in a study session, not in a practice, which already writes nothing (`card_practice_offered`).
- **The existing practice, with a one-card queue**, not a new study scope. `Store::practice_card(card_id)` returns that card as the queue, read under the same `query_only` guard as `practice_queue`, and refuses what a deck's practice leaves out: `CardNotFound` for a deleted card, the new `CardSuspended`, `DeckArchived` and `DeckNotFound`. The page is `ReviewPage::practice_card`, a practice of the card's deck (its title, its scope for `practisable_card`), marked `one_card`.
- **Done is the answer.** After Show Answer, Pass or Fail ends it: a Fail does not bring the card back (the queue holds nothing else), and the page goes back to where it was started, with no summary. "Practice Complete" over one answer that changed nothing would tell nothing, and a second click to leave it would be a step for no reason (`at_end`). A Suspend, Delete or Move from the practice's own Card Menu also ends it the same way; the page leaves once the main loop is idle, so the Undo toast is shown first and still works.
- **The focus comes back to the card's row.** The deck page and All Cards remember the card they pushed a practice of and, when shown again, focus its row (`ui::focus::back_from_practice`); without the row (deleted or moved away meanwhile) they focus as before. The navigation view's own restore is not enough: it gives back the row's menu button, and a recycled row may show another card by then. From a study session, the practice is pushed above it; coming back, the session's card is focused again, and a card suspended, deleted or moved out of the session's scope in the practice leaves the session too. The toast's Undo of that Suspend, Delete or Move then restores only the database, not the card in the session beneath: the practice page that made the toast has left the window by then, which is SPEC's rule for an Undo after its page is gone.
- **No shortcut.** The entry is in the menus only.

## Consequences

The no-write guarantee is unchanged: the tests check, as for a deck's practice, that the database files, the log and another connection's `data_version` are untouched by practices of one card (core: `a_cards_practice_leaves_the_database_byte_identical`; app: `a_practice_of_one_card_writes_nothing_and_goes_back_after_an_answer`). All Cards' rows now know whether their deck is archived (`BrowseRow::deck_archived`). A one-card practice records no time and has no summary, by design.
