# 81. S starts studying; 3 and 4 pass

- Status: Accepted
- Date: 2026-11-09
- Refers to: Phase 3b brief, slice 6; ADR 0037 (keys of the review page), 0064 (key hints in tooltips), 0065 (the deck page's keys)

## Context

Studying is the main thing Rappel does, but starting a session needed the mouse or Tab to the Study button, and Anki users reach for 3 and 4 (Good, Easy) as well as 2.

## Decision

- **S is decided by a pure function.** `ui::keys::starts_studying(key, modifiers, text_focus)` is true only for a plain S (or Caps Lock's S) with no text field focused. Ctrl, Alt, Shift, Super, Meta and Hyper make it another shortcut (Shift+S is Suspend in review) and give false.
- **S acts on its release.** A held key repeats as further key presses with no release between (`key-pressed` and `key-released` of `GtkEventControllerKey`, gtkeventcontrollerkey.c). Starting on the press put the session on screen under the repeats, and the session's own S (Suspend) suspended cards one after another (found in review: six cards in 2.5 s). So `ui::keys::StartHold` swallows the press and every repeat on the page that stays, and the first release starts the session once, if no text field has the focus by then.
- **Deck page:** S activates `deck.study`, so it does nothing when the Study button is disabled (nothing to study) and while cards are being chosen. **Deck list:** S on a focused deck row (the row, or its menu button) pushes (on the release) a session for that deck, started from the deck list ("Back to Decks"), without pushing the deck page first; a deck with nothing to study does nothing, as its page's disabled button would.
- **Review page and practice:** 3 and 4 are added to the page's key table beside 2, so they pass exactly as 2 does: the same settle delay against key repeat, no effect before the answer is shown, and in a practice nothing is rescheduled. The grades are not decided by the pure function: the page binds them as GTK shortcut triggers (so an AZERTY keyboard, where digits need Shift, behaves as GTK maps it), in the bubble phase, where a focused text field has already taken the key; a test checks the table.
- **Shown:** the Study buttons' tooltip is "Study (S)" (the translatable `{name} ({key})` of ADR 0064). The Keyboard Shortcuts dialog gets Start Studying (S) and Pass lists 2, 3, 4 and Space; its test checks both by name, since S is also Suspend in review.

## Consequences

3 and 4 do not grade differently: FSRS here has two grades. The accessible descriptions of the grade buttons still say "Press 2 or Space"; they stay true.
