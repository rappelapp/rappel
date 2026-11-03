# 65. One selection mode for All Cards and the deck page

- Status: Accepted
- Date: 2026-11-02
- Refers to: SPEC §4.6; `docs/briefs/phase-3a.md` (slice 2, #104); ADR 0036 (All Cards), ADR 0044 (rows), ADR 0058 (phone and touch), ADR 0064 (Ctrl+A, Move to Deck…)

## Context

All Cards has a selection mode (ADR 0036): a Select button, a header that says "3 Selected" with Select All and Cancel, an action bar (Move…, Tags, Suspend or Unsuspend, Reset, Delete), the dialogs those actions ask with, the writes, and an Undo toast for each. The deck page needs the same (#104). The brief asks for one shared component first, so the two pages cannot drift, with All Cards' behaviour and tests unchanged, and for the choice between a composite widget with its own `.blp` and a controller object to be recorded here.

The two pages differ in what they list and how they read it: All Cards replaces its whole list from a search and filters (`BrowseItem`), the deck page keeps one deck's cards in creation order and updates them item by item (`CardObject`). Their headers differ too (a search field; the deck's name, New Card, Study and the Deck Menu). Selection mode's parts are in three places on the page: the header, the list's rows, and a bar at the bottom.

## Decision

**A controller object, with the layouts it drives in two small composite widgets.** One composite widget cannot hold both the header and the bottom bar: they are separate top and bottom bars of the page's `AdwToolbarView`. A controller alone would leave each page with its own copy of the header buttons and the action bar in Blueprint, which is where they would drift. So:

- `ui::selection::CardSelection` (the controller, no layout) owns the chosen cards (`chosen::Selection`, as before), the switch between the page's own header and the selection header (and `can-pop`, so Escape leaves the mode and Back hides), the action bar's state (which actions are enabled, which way Suspend goes, the title), the dialogs (Move, Add Tags, Remove Tag, from `browser_dialogs`), the writes through `AppState` (`bulk::perform`, moved from the page with its tests), the Undo toasts, Ctrl+A (`ui::select_all`), Space on the list, and where the focus goes after an action (`ui::focus::index_after_removal`).
- `widgets::SelectionHeader` (`data/selection_header.blp`) is an `AdwHeaderBar` with Select All, the title and Cancel. The page puts it as a second top bar, hidden; in selection mode it shows and the page's own header hides. Swapping the whole header keeps each page's normal header its own and the selection header the same on both.
- `widgets::SelectionBar` (`data/selection_bar.blp`) is the `GtkActionBar` with the five buttons and the Tags menu.
- Both have a `compact` property: on a phone the buttons keep only their icons (ADR 0058). All Cards binds it to its 500 sp breakpoint as before. The deck page has a 400 sp breakpoint already (Study moves to a bottom bar), so it adds one at 500 sp before it; only the last matching breakpoint applies (`AdwBreakpointBin` docs), so the page ORs the two.
- The actions are a `GSimpleActionGroup` named `selection` inserted on the page, so the page's Select button, the shared header and the shared bar all reach them: `selection.choose`, `cancel`, `select-all`, `move`, `add-tags`, `remove-tag`, `suspend`, `reset`, `delete`.
- The page owns its list and what fills it, and answers the rest through the trait `SelectionPage`: what to do when the mode changes (its rows follow a `selecting` property), when cards were written (All Cards reads its search again; the deck page reads its cards again and updates the counts), whether the list is shown, how to focus its empty state, and whether another deck exists. The items implement `SelectableCard` (id, suspended, new, tags, and a `selected` property the row follows); `CardObject` gains that property and keeps its tags for Remove Tag….
- `CardRow` follows `selected` on either item type, shows its check button instead of its menu in selection mode on both pages, and starts selection mode on a long press on both.

**The refactor came first, in a commit of its own, with no change in behaviour.** The All Cards tests stayed; those of the pure parts moved with them (`bulk.rs`, `text.rs`), and those of the page reach the header and bar through the new widgets instead of the page's own fields, with the same assertions.

**Then the deck page, and four changes the second page brought to both:**

- **Move… offers every deck but the page's own** on the deck page (`move_targets`), and is disabled when there is no other deck, on both pages, like the rows' Move to Deck… (`move_card::offered`). All Cards enabled it with one deck before, where it could only say "Nothing changed".
- **A list that becomes empty ends selection mode** and puts the focus on the empty state's button (the deck page's Add Card, All Cards' status page button).
- **The focus after a dialog's action.** `AdwDialog` gives the focus back, when it closes, to the widget that had it: the action bar's button, hidden by then. The focus an action set on the response (the card it started with, or the row now at its place) is set again when the dialog has closed, as ADR 0064 did for Move to Deck….
- **A button of the action bar that had the focus** when the mode ends gives it to the Select button, as Select All and Cancel already did.
- **The controller follows the list, not the page.** It listens to the list's `items-changed`: an item put in the list (read again, or a row replaced after a suspend, a reset, an edit or an Undo) shows whether its card is chosen, and cards that left the list are dropped from the choice. A page cannot forget to do it (the review found a deck page row replaced by an Undo that showed unticked while still counted).
- **The focus moves only on the page the navigation view shows**, so a page under another one (an Undo from a later page) does not take it.
- **A long press while choosing** chooses or drops the card, as a tap does.

On the deck page: a Select Cards button in the header (`selection-mode-symbolic`, tooltip and accessible label "Select Cards", HIG "Selection mode"), between Study and the Deck Menu, disabled while the deck is empty; a click or Enter on a row chooses it in selection mode instead of opening the editor; Ctrl+N and Ctrl+F do nothing while cards are chosen (as Ctrl+F on All Cards); Escape leaves the mode, then goes back as before (`escape_action`, shared with All Cards and moved to `ui::selection::text`); the Study bar of a narrow window gives way to the action bar; moved and deleted cards leave the list; Undo brings the list back. The Keyboard Shortcuts dialog has a Deck section with Select All, Select or Deselect Card and Stop Selecting.

## Consequences

- The two pages share every text, dialog, write and Undo of selection mode; a change to it is made once.
- A third page that lists cards (study by tag, an archived deck's cards) gets selection mode by implementing `SelectionPage` and `SelectableCard`.
- The controller is generic over the page and the item type; the pages hold it in a `OnceCell` set in `new`.
- At 360 px with the minimise and maximise buttons shown, the deck page's title and a long selection title ("39 sélectionnées") are shortened with "…", as All Cards' already was; with GNOME's default window buttons they fit (phase 3a's leftovers slice looks at header titles at 360 px).
