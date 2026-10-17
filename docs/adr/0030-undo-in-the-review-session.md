# 30. Undo in the review session, and Reset Progress

- Status: Accepted
- Date: 2026-10-17
- Amends: ADR 0014 (what a `Session` remembers)
- Closes: #38; the UI part of SPEC §4.5 (undo last grade)

## Context

ADR 0028 gave the log `Store::undo` and `Store::reset_card`. The app has to use them: Ctrl+Z on the review page, and Reset Progress on the deck page.

## Decision

**The session takes an answer back exactly.** `Session::graded` now also takes the id of the log entry `record_review` made, and remembers what the call changed (where the card was in the queue or the steps, whether it added a step, whether it was seen for the first time, the grade and the capped time). `undo_last` applies the inverse, newest answer first, so each inverse is exact. A property test checks that any run of answers and undos leaves the same summary and remaining count as a session that only got the surviving answers. The page first undoes the entry in the log (`AppState::undo`), and only on success undoes it in the session.

**The undone card is shown next.** `undo_last` pins the card: `next` returns it before anything else, even a step card that became due meanwhile. A later answer or skip of that card clears the pin. The page reads the card again, so an edit made during the session shows, and shows it on its front side with a new timer.

**Where it works.** Ctrl+Z is a bubble-phase shortcut of the review page, like the other keys, and also works on the summary. It is not held back by the key-repeat guard, because it grades nothing. It is ignored while the card editor is open. A header button (tooltip and accessible label "Undo Last Answer") is enabled while the session has an answer to take back. The session, and with it the history, ends when the page is left.

**A stale undo changes nothing.** If the store refuses (`NotLatestEntry`, `EventNotFound`, `CardNotFound`: the card's log changed, e.g. by an import), the page shows "This answer can no longer be undone" and forgets the whole history, since older answers can only be undone after this one. Other errors keep the history and show an error toast. `ui::undo::failure` makes this decision for the review page and the reset toast.

**Reset Progress has no confirmation.** It is in the card's menu and takes effect at once, because nothing is lost (the history stays in the log) and a toast offers Undo, which undoes the reset entry. If the card was studied since, the toast's Undo says "The reset can no longer be undone" and writes nothing. After a reset or its undo, the deck page and the deck list update their counts (`RappelWindow::study_changed`).

## Consequences

- Undoing a Fail on a card that went into the 10-minute step takes it out of the step again; undoing a card's second answer puts it back in the step it had.
- Undo entries pile up in the log (one per undone answer). That is how ADR 0028 keeps the log append-only.
- The undo works only for answers of this session. Undoing an older review needs the card information screen (#39).
