# 13. Deck page, card list and card editor

- Status: Accepted
- Date: 2026-10-15
- Amends: ADR 0012 (deck rows become activatable; Ctrl+N)

## Context

The deck list exists (ADR 0012), but a deck cannot be opened. A deck needs a page with its cards, a way to add and edit them, and a delete that can be undone, like decks. A deck can hold thousands of cards, so the list must stay smooth at 5,000. SPEC §4.4 describes the final editor (GtkSourceView, Markdown highlighting, preview); this slice is the plain-text first step.

## Decision

**`Store::undelete_card(id)`.** One immediate transaction:
- An unknown id gives `CardNotFound`. A live card is returned unchanged, so a second click on Undo is harmless.
- If the card's deck is deleted, it fails with `DeckNotFound` and changes nothing. A live card in a deleted deck would be invisible and would come back with neither the card nor the deck undo. Undoing the deck does not bring this card back either: its stamp is older than the deck's (ADR 0008, 0012). Once the deck is back, the card can be restored.
- Otherwise `deleted_at` becomes NULL. `updated_at` is not touched, because delete did not touch it.

**Navigation.** Activating a deck row pushes a new `DeckPage` onto the window's `AdwNavigationView`, which gives the back button, swipe-back and Alt+←. The page title is the deck name (plain text). When the deck list is shown again (`AdwNavigationPage::shown`), it reloads and focuses the deck that was open, so its new/due counts are current.

**The card list is a `GtkListView` with a model**, not a boxed `GtkListBox`. A list box makes one widget per card. A list view makes widgets only for the rows on screen and recycles them while scrolling.
- The model is a `gio::ListStore` of `CardObject`. Each holds the `Card` and its preview, computed once.
- The preview is the front on one line: whitespace runs (newlines included) become one space, and it is cut at 200 characters. A label lays out all its text even when it ellipsises, so a huge front is cut before it reaches the label. The label ellipsises the rest at the end, with markup off.
- Changes update the model item by item, instead of rebuilding 5,000 items: an edit replaces one item, and an add or Undo inserts in creation order. Card ids increase with creation (ADR 0008), the same order as `list_cards` (`seq`), so the insert compares ids. A delete removes one item. Scroll position is kept, and the focus moves to the affected card or its neighbour.
- **Styling.** libadwaita's `.boxed-list` only styles `GtkListBox` (its selectors are `list.boxed-list`). For a list view, libadwaita provides `.rich-list` (row padding and height) and `.separators`. So the list is flat rows with separators in an `AdwClampScrollable` column (max 600 px), not rounded cards like the deck list. The scrolled window has the `view` background, so the column's background is not a strip on a darker background.
- A click or Enter on a row opens the editor (`single-click-activate`, with `NoSelection`). Each row has a menu button (Edit…, Delete) whose items forward to page actions with the card id, as `DeckRow` does. The list item's accessible label is the preview.
- `fill_scratch_db` (a `cards-core` example) creates a deck of 5,000 varied cards in a scratch directory, so the list can be tried. It refuses the app's own data directories.

**Delete and Undo.** A delete is immediate, with a high-priority toast "Card deleted" and Undo. The front is not in the toast: it can be long or several lines. The Undo closure holds the `AppState`, not the page. So Undo still works after going back to the deck list, when the page may be gone. It refreshes the deck list's counts too, and if the deck was reopened meanwhile (a new page), it puts the card back on the visible page.

**Card editor: an `AdwDialog`.** A dialog keeps the list visible behind it, and it becomes a bottom sheet on narrow windows. The header bar has Cancel and a suggested Save. Front and back are `GtkTextView`s with word wrapping, in `.card` frames, with `accepts-tab: false` so Tab moves the focus.
- Ctrl+Enter saves and closes. Ctrl+Shift+Enter saves, clears the fields, keeps the same deck, puts the focus in the front field and shows "Card added" ("Card saved" when it was an edit) in the dialog's own toast overlay. Saving an unchanged card writes nothing, so `updated_at` is not bumped. The shortcuts are in a capture-phase `ShortcutController` on the dialog, so they run before the text view treats Enter as a new line. They always consume the key, so with a blank front Ctrl+Enter does nothing instead of adding a line.
- A blank front disables both save actions and the Save button. This is the store's own rule (`trim().is_empty()`).
- A store error shows a toast in the dialog. The dialog stays open, with the text as typed.
- Closing with unsaved changes (Cancel, Escape) asks "Discard Changes?". Losing typed text silently would be a small data loss. The dialog's `can-close` is off while the text differs from the last saved text.
- Only one editor is open at a time, and it reports each save to the page.

**Ctrl+N.** It is bound to `win.new`, which looks at the navigation view's visible page: a deck page opens the card editor, and the deck list opens the name dialog. Buttons use their own actions (`win.new-deck`, `deck.new-card`).

## Consequences

- The card list looks different from the deck list (flat rows, not boxed). If libadwaita adds boxed styling for list views, it can be adopted.
- Undo lasts as long as the toast. As with decks, a deleted card stays in the database until a trash feature exists.
- The editor slice (SPEC §4.4) replaces the text views with GtkSourceView and adds the preview. The shortcuts and the save rules stay.
- The discard question covers Cancel and Escape. Closing the whole window while the editor has unsaved text still loses it; the window's `close-request` can handle that later.
- Inserting in order assumes id order equals `seq` order. The import slice (SPEC §4.1) must keep that true, or the list must compare by `seq`.
- `Store::edit_card` now returns the stored card (one `UPDATE … RETURNING`), so the app never reports a committed edit as failed.
- The deck page holds a copy of each card's text. A deck page shows one deck, and only this app writes the database (ADR 0011), so the copy cannot go stale while the page is open.
