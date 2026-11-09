# 81. S starts studying; 3 and 4 pass

- Status: Accepted
- Date: 2026-11-09
- Refers to: Phase 3b brief, slice 6; ADR 0037 (keys of the review page), 0064 (key hints in tooltips), 0065 (the deck page's keys)

## Context

Studying is the main thing Rappel does, but starting a session needed the mouse or Tab to the Study button, and Anki users reach for 3 and 4 (Good, Easy) as well as 2.

## Decision

- **One pure function decides.** `ui::keys::study_key(key, modifiers, text_focus)` gives Start (S), Fail (1), Pass (2, 3, 4) or nothing. The keypad's digits count as the main row's. Ctrl, Alt, Shift and Super make it nothing (Ctrl+S, Shift+2 keep their own meanings); Caps Lock and Num Lock do not count as modifiers. A text field with the focus makes it nothing.
- **Deck page:** S activates `deck.study`, so it does nothing when the Study button is disabled (nothing to study) and while cards are being chosen. **Deck list:** S on a focused deck row (the row, or its menu button) pushes a session for that deck, started from the deck list ("Back to Decks"), without pushing the deck page first; a deck with nothing to study does nothing, as its page's disabled button would.
- **Review page and practice:** 3 and 4 are added to the page's key table beside 2, so they pass exactly as 2 does: the same settle delay against key repeat, no effect before the answer is shown, and in a practice nothing is rescheduled. The review page keeps GTK shortcuts in the bubble phase, where a focused text field has already taken the key; a test checks the table against `study_key`.
- **Shown:** the Study buttons' tooltip is "Study (S)" (the translatable `{name} ({key})` of ADR 0064). The Keyboard Shortcuts dialog gets Start Studying (S) and Pass lists 2, 3, 4 and Space; its test checks both by name, since S is also Suspend in review.

## Consequences

3 and 4 do not grade differently: FSRS here has two grades. The accessible descriptions of the grade buttons still say "Press 2 or Space"; they stay true.
