# 12. Deck list and undoing a deck delete

- Status: Accepted
- Date: 2026-10-15
- Amends: ADR 0008 ("There is no undelete in this slice")

## Context

The main page needs the list of decks, with create, rename and delete. SPEC §1 puts data first, and the GNOME HIG prefers undo over confirmation for actions that can be undone. ADR 0008 already gave a deck and its cards one shared `deleted_at` stamp so that a delete could be undone exactly, but no undelete existed.

## Decision

**`Store::undelete_deck(id)`.** One immediate transaction:
1. Read the deck's `deleted_at` stamp `s`.
2. Make the deck live again.
3. Make live again exactly the cards of that deck with `deleted_at = s`.

Cards deleted on their own before the deck have a smaller stamp (ADR 0008 makes the deck's stamp strictly greater), so they stay deleted. `updated_at` is not touched, because delete did not touch it either.
- A deck that is already live is returned unchanged. A second click on Undo is harmless.
- If a live deck took the name in the meantime, the call fails with `DuplicateDeckName` and the transaction rolls back. The user gets a toast, and the deck stays deleted with its data intact.
- An unknown id gives `DeckNotFound`.

**Delete without confirmation, undo from a toast.** The deck disappears at once, and a toast "“Name” deleted" offers Undo for the toast's normal timeout. The toast has high priority, so it is shown at once even when a toast that stays until dismissed (a backup warning, an error) is on screen. Nothing is ever removed from the database, so a deck whose toast was missed is still recoverable by a later trash feature.

**Layout.**
- The window's main page is an `AdwNavigationView` with one root `AdwNavigationPage` ("Cards"). The next slice pushes a deck page onto it, which gives the back button, swipe-back gesture and Alt+← for free.
- The decks are a `.boxed-list` in an `AdwClamp` (max 600 px). Each row is an `AdwActionRow`: the name is the title, with markup off because names are untrusted, and "N new · M due" is the subtitle.
- Each row has a flat menu button (`view-more-symbolic`) with Rename… and Delete. Its tooltip is "Deck Actions", and its accessible label names the deck.
- Rows are not activatable until opening a deck exists.
- "New Deck" is a header-bar button (tooltip and accessible label), a pill button on the empty `AdwStatusPage`, and Ctrl+N. All three use the action `win.new-deck`, which is enabled only once the deck list is shown.
- **At 360 px** nothing needs a breakpoint. The clamp follows the window width, titles ellipsize, the counts sit on their own line, and `AdwAlertDialog` becomes a bottom sheet.

**Name dialog.** An `AdwAlertDialog` with an `AdwEntryRow`. The name is checked as it is typed, with the core's `deck_name_key`, so the check is the same rule as the store's:
- An empty name disables the confirm button.
- A duplicate disables it too, puts the entry in error style, and says "A deck named “X” already exists" under the entry. The entry's accessible description is that label.
- Rename skips the deck itself, so a case-only rename is allowed.

The store still has the final say; if it refuses, a toast says why. Only one name dialog can be open at a time: a double click or Ctrl+N while one is open does nothing.

**Row actions.** The row menu is a static menu model in Blueprint. Its items activate the row's own `row.rename` / `row.delete` actions. These forward to the page's `decks.rename` / `decks.delete` with the row's deck id. After each change the list is rebuilt from the store, and focus goes to the affected row, or to its neighbour after a delete, so keyboard users keep their place.

**Logs.** Deck names are user content, so operation failures are logged at `warn` without the message, and in full only at `debug` (SPEC §2).

## Consequences

- Undo lasts as long as the toast. A deleted deck that was not undone stays in the database, invisible, until a trash or restore feature exists.
- Rebuilding the whole list on each change is fine for tens of decks. A `gio::ListModel` can replace it if lists grow large.
