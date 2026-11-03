# 64. Small polish: key hints in tooltips, Ctrl+A, Move to Deck…, tables

- Status: Accepted
- Date: 2026-11-02
- Amends: ADR 0045 (the key hints under the review page's buttons are gone)
- Refers to: SPEC §4.5, §4.6; `docs/briefs/phase-3a.md` (slice 1); ADR 0036 (All Cards), ADR 0037 (suspend during review), ADR 0058 (phone and touch)

## Context

Four small items of the phase 3a brief: #105, #106, #102 and the table width of PR #32.

## Decision

### Key hints leave the review page (#105)

ADR 0045 put a dimmed caption ("Space", "1", "2") under each answer button. The maintainer finds them cluttered. They are gone, with the breakpoint setters that hid them on short windows. Each button (`reveal_button`, `fail_button`, `pass_button`) says its key in its tooltip instead: "Show Answer (Space)", "Fail (1)", "Pass (2)". The format is one translatable string with named placeholders, `{name} ({key})`, filled by the pure `key_tooltip`; the key's text is how GTK labels the first trigger of the page's `KEYS` table bound to that key (`gtk_accelerator_get_label`, "Space" in the user's language), so the tooltip cannot name a key the page does not bind. The accessible descriptions ("Press 2 or Space") stay, and the Keyboard Shortcuts dialog is unchanged. ADR 0045 said the tooltips were dropped because the key was on screen; now that it is not, they come back. A touch user sees no tooltip, and needs none: grading is by the buttons.

The store screenshots in `data/screenshots/` still show the hints. They are retaken with the release.

### Ctrl+A chooses every card listed (#106)

All Cards already chose every card on Ctrl+A, but only while the card list had the focus. Now the key works from anywhere on the page and enters selection mode if the page is not in it (`BrowserPage::select_all`, which also does nothing when no card is listed). It chooses the cards the filters list, not the whole collection. If the Select button had the focus when the mode started (it hides), the focus goes to the card list, or to Cancel when there is no list. This happens whenever the mode starts with the focus there, so pressing Select with Enter or Space moves the focus to the list too, as it did before through `start_choosing`.

The key is handled by `ui::select_all::install`, a helper the deck page will use for its own selection mode (slice 2). **Phase and scope.** GTK binds Ctrl+A in `GtkText` (selects all the text: "Shortcuts and Gestures" of the `GtkText` docs) and `GtkListView` (`list.select-all`, `GtkListBase` docs). A controller in the Capture phase on the page sees the key first (capture runs from the root down to the target, then the event bubbles up), so it would also take the search entry's Ctrl+A. Instead of relying on the text field to stop the key later, the pure `selects_cards` leaves the key alone when the window's focus widget is a `GtkEditable` or a `GtkTextView`, and when the event comes from a popover (a drop-down's popup has its own surface and focus). Capture is what keeps the list view's own bindings from seeing it. The unit tests cover the decision; harness tests press the page's controller with the list, the Select button and the search entry focused; the key was also tried with real key events under Xvfb (the screenshot harness).

The Keyboard Shortcuts dialog already listed "Select All" with All Cards' keys; its test now reads the key from `ui::select_all::KEYS`.

### Move to Deck… for one card (#102)

An entry in each card's ⋮ menu on the deck page, in All Cards' row menu (the two lists share the `CardRow` and its menu), and in the review page's Card Menu next to Suspend. All three use `dialogs::move_card`: the chooser is All Cards' (`browser_dialogs::choose`, now with an `on_closed` hook), the move and its Undo are core's `move_cards` and `unmove_cards` through `AppState`, and the toast is "Card moved" with Undo.

- **The card's own deck is not offered** (`move_card::targets`). **No other deck:** the entry is disabled (`move_card::offered`; a menu item naming a disabled action is insensitive), from the page's `can-move` property on the lists and from the card shown on the review page. If the last other deck goes while the menu is open, the chooser is not shown and a short notice says so.
- **Deck page:** the card leaves the list; the focus goes to the row now at its place, else the last one, else Add Card (`ui::focus::index_after_removal`); Undo brings the row back in its place (`restored`) and the other pages read the change (`RappelWindow::card_restored`). `AdwDialog` gives the focus back to the widget that had it when it closes, after the response, which would put it on a recycled row's menu button, so the focus is set again once the chooser has closed (the chooser's `on_closed`; ADR 0037 has the same for the delete question).
- **All Cards:** the move is the bulk Move with one card (`run_on`), with its toast "Card moved", the same as on the deck page and in review (`Bulk::MoveOne`; the bulk Move of selected cards keeps "N cards moved"), and its focus and Undo are the bulk action's.
- **Review:** as Suspend (ADR 0037). The card leaves the session (`Session::skip`, not counted as answered), the next card or the summary shows, and the pages under the session read the change. The keys are ignored while the chooser is open (`asking`, as for the delete question), and the focus is parked on the card when it closes. Undo moves the card back and, if it went, puts it into the session as the next card (`Session::put_back`); an Undo clicked after leaving the page changes the database only. If the card could not go back (moved again, deleted, or its first deck deleted since) a notice says so. A card deleted or already moved elsewhere leaves the session with a notice and nothing is written. Move to Deck… has no key: a letter key for a dialog that needs a deliberate choice would be pressed by mistake.
- Archived decks do not exist yet; the archive slice leaves them out of `targets`.

### A table's border hugs its columns (PR #32, ADR 0019)

PR #32 was written against the first renderer and is redone here on the current one (`render::table`, which now also takes the side's math budget). A table's `GtkGrid` sits in a horizontal `GtkScrolledWindow`. The viewport gives its child at least its own width (`hscroll-policy` is `Minimum`, `GtkScrollable` docs), and the grid's default `Fill` alignment stretched the bordered grid across the whole card, leaving an empty strip inside the border on a wide window. The grid now has `halign: Start`, so it keeps its natural width (`GtkAlign` docs: alignment applies when a widget is given more than it needs). A table wider than the window still gets its full width, since its cells do not wrap, and scrolls. #32 can be closed.

## Amended in phase 3a (slice 7): the card editor's hint, and an audit (#117)

The maintainer found the same clutter in the card editor: a dimmed "Ctrl ⏎ to save" (an `AdwShortcutLabel` and a caption) after the fields. It is gone, with the box that held it (`save_hint`). **Save** says its key in its tooltip, "Save (Ctrl+Enter)", through the same `{name} ({key})` string and GTK's accelerator label as the review page's buttons, and its accessible description keeps stating it ("Press Ctrl+Enter"). The helpers moved from the review page to `ui::keys` (`tooltip`, `label`), so any page can use them; the review page's `KEYS` table stays its own. There is no button for Save and Add Another (Ctrl+Shift+Enter), so it needs nothing. The Keyboard Shortcuts dialog is unchanged. ADR 0043's "quiet hint" is superseded.

**The audit.** Every `.blp` file and every `gettext` call was searched for a key shown on screen (`Ctrl`, `⏎`, `Space`, `Enter`, `Esc`, "Press", "to save", `AdwShortcutLabel`, `.dimmed` captions):

| Where | Old | Now |
|---|---|---|
| `card_editor.blp` | visible "Ctrl ⏎ to save" under the tags | removed |
| `card_editor.blp`, Save | tooltip "Save (Ctrl+Enter)", fixed English | tooltip from `ui::keys` and the editor's `SAVE_KEYS`; description "Press Ctrl+Enter" |
| `review_page.blp`, Undo | tooltip "Undo Last Answer (Ctrl+Z)", fixed English | `ui::keys` from the page's `KEYS`: "Undo Last Answer (Ctrl+Z)" in the user's language |
| `review_page.blp`, Edit | tooltip "Edit Card (E)", fixed English | `ui::keys` from `KEYS`: "Edit Card (E)" |
| `review_page.blp`, Show Answer / Fail / Pass | accessible descriptions "Press Space", "Press 1", "Press 2 or Space" | unchanged (#105: descriptions, not visible) |
| `card_editor.blp`, Tags group | description "Press Enter or type a comma after each tag" | kept: it says how the field works (a comma or Enter makes a tag), not a shortcut for a control |
| Add Tags dialog body | "…Press Enter or type a comma after each one." | kept, same reason |
| tag suggestions (`tag_entry/keys.rs`) | the announcement to screen readers "Tab takes the first, the arrow keys choose, Escape keeps what you typed" | kept: spoken, not shown |
| menus (`card_menu`, header menus) | GTK's accelerator labels ("I", "S", "Delete", "E", "Ctrl+,") | kept: GTK's own, as asked |

Nothing else shows a key. The store screenshots in `data/screenshots/` that show the editor, if any, still show the old hint; they are not retaken here.
