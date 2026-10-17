# 37. Suspend and delete during review

- Status: Accepted
- Date: 2026-10-17
- Amends: SPEC §4.5 (card actions during review)
- Refers to: ADR 0013 (card deletion), ADR 0014 (study sessions), ADR 0030 (Ctrl+Z on the review page), ADR 0035 (suspension), ADR 0036 (the toast-with-Undo pattern)

## Context

SPEC §4.5 lists Edit, Suspend and Delete with confirmation as card actions during review. Edit existed. Suspension and the lenient Undo calls arrived with ADR 0035, and the deck page and All Cards had Suspend and Delete. A card that is wrong, obsolete or a leech should be dealt with where it is found, without leaving the session.

## Decision

**Where.** The Card Menu of the header (next to Card Information) has Suspend Card and Delete Card…. **S** suspends and **Delete** deletes, at any time, before or after the answer is shown. Neither grades anything, so the settle delay that stops key repeat from grading does not apply, but key repeat needs a guard of its own: after the first S the next card is on screen, and holding the key would suspend one card after another. So **S needs the card to have been on screen for the same 300 ms** (`suspend_allowed`); the menu item is not delayed. Delete is guarded by its question (`asking`), which opens on the first press. The page ignores both keys while the editor or the delete question is open.

**Suspend** writes at once (`set_suspended`), takes the card out of the session (`Session::skip`: it is not counted as answered and nothing is added to the log), shows the next card or the summary, and offers "Card suspended" with Undo.

**Delete asks first.** An `AdwAlertDialog` ("Delete Card?") with Cancel, which is the default and the close response, and a destructive Delete. Enter and Space never delete. The question is about the card that was on screen when it opened; if the page has moved on it does nothing. After Delete, the card leaves the session as for a suspend, with "Card deleted" and Undo. The body says that the review history is kept and Undo brings the card back.

**The toast's Undo** puts the flag back (`restore_suspended`) or brings the card back (`undelete_card`), then, if the review page is still shown, puts the card back into the session with the new `Session::put_back`: it goes to the front of the queue and comes next (a card an undo brought back, or a step card that is due, still goes first; a card that waited in a step comes back through the queue, so it may be shown before its step is due), and if the summary was on screen the card replaces it. An Undo clicked after leaving the page restores the card in the database only, and asks the window to read the deck pages and the lists again. It keeps the state and weak references, not the page. The card is put back only if it is live and not suspended by then. If the Undo fails (an undelete into a deleted deck), a persistent error toast says so. A suspend of a card that was already suspended elsewhere offers no Undo. After the action and after its Undo, `RappelWindow::data_changed` makes the deck page under the session, the deck list and an open All Cards page catch up; the session keeps its own cards.

**Ctrl+Z is for answers, the toast is for removals.** They stay separate: Ctrl+Z walks back the session's answers newest first (ADR 0030). Two cases follow from removals:
- The answered card was deleted since: `Store::undo` says `CardNotFound`, which is already a stale Undo ("can no longer be undone").
- The answered card was suspended since (here, or from another window mid-session): the answer is taken back in the log, and the session then skips any card that is suspended when its turn comes (`show_next`), so Ctrl+Z never shows a card that is not studied. The same rule skips a card suspended elsewhere before its turn, silently.

If the card is already gone when the action runs (deleted elsewhere), it leaves the session with a notice and nothing is written.

## Consequences

- `Session::put_back` is the one new core call. It leaves the card in front of the queue and does nothing for a card the session already holds, so a repeated Undo is harmless; `Session::undo_last` no longer queues a card twice if it was put back since.
- After each action `data_changed` reads the deck page's whole list again (a few tens of milliseconds at 5,000 cards), which is fine once per action now that S cannot repeat.
- A suspended card that had been answered in this session stays in the summary's counts; the answer happened.
- Delete is a key as well as a menu item. The question is the guard, and Cancel is the default.
