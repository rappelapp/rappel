//! Every text and small decision of selection mode that needs no GTK, the
//! same on All Cards and on a deck's page (`ui::selection`): the header's
//! title, the action bar's Suspend button and Reset Progress, the dialogs'
//! texts, the Undo toasts and their notes, what a screen reader adds to a
//! row, and what Space does on the list. Pure, so it is unit-tested
//! (SPEC §4.6, ADR 0036, ADR 0065).

use crate::i18n::{fill, gettext, ngettext};
use crate::ui::text::{counted, group_digits};

/// What a screen reader says for a row (`base`), with, while cards are
/// being chosen (`selected` is `Some`), whether it is chosen: "2 + 2,
/// back: 4, selected".
pub fn with_selected(base: String, selected: Option<bool>) -> String {
    match selected {
        // Translators: {row} is a card as a screen reader says it, while
        // cards are being chosen.
        Some(true) => fill(&gettext("{row}, selected"), &[("row", &base)]),
        // Translators: {row} is a card as a screen reader says it, while
        // cards are being chosen.
        Some(false) => fill(&gettext("{row}, not selected"), &[("row", &base)]),
        None => base,
    }
}

/// The header's title in selection mode (HIG, "Selection mode": the
/// heading says how many items are chosen): "3 Selected", or "Select
/// Cards" before the first.
pub fn selection_title(n: usize) -> String {
    if n == 0 {
        gettext("Select Cards")
    } else {
        // Translators: the title while cards are being chosen; {count} is
        // how many are.
        counted(&ngettext("{count} Selected", "{count} Selected", n), n)
    }
}

/// The header's title where "3 Selected" would be cut: the bare count
/// ("3"; the full title is the tooltip and the screen reader's label).
/// Before the first card is chosen it stays "Select Cards", which says what
/// the mode is for and is short enough.
pub fn selection_title_tight(n: usize) -> String {
    if n == 0 {
        selection_title(0)
    } else {
        group_digits(n)
    }
}

/// The window width in pixels under which the header shows [`selection_title_tight`]:
/// at 360 px, with minimise and maximise shown beside Select All and
/// Cancel, the title has about 90 px, which cuts "30 sélectionnées"
/// (ADR 0071).
pub const TIGHT_HEADER_WIDTH: i32 = 400;

/// Which way the Suspend button of the action bar goes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SuspendChoice {
    Suspend,
    Unsuspend,
}

/// Unsuspend only when every chosen card is suspended; otherwise Suspend,
/// which leaves the suspended ones as they are.
pub fn suspend_choice(suspended: impl IntoIterator<Item = bool>) -> SuspendChoice {
    let mut any = false;
    for flag in suspended {
        if !flag {
            return SuspendChoice::Suspend;
        }
        any = true;
    }
    if any {
        SuspendChoice::Unsuspend
    } else {
        SuspendChoice::Suspend
    }
}

/// Reset Progress is offered while a chosen card has progress (not every
/// one is new); the reset leaves the new ones alone.
pub fn reset_offered(is_new: impl IntoIterator<Item = bool>) -> bool {
    is_new.into_iter().any(|new| !new)
}

/// The note when a reset of `n` cards found them all new already.
pub fn no_progress_text(n: usize) -> String {
    ngettext(
        "This card has no progress to reset",
        "These cards have no progress to reset",
        n,
    )
}

/// A key of the card list's own in selection mode (besides Escape, which
/// the page takes, and Ctrl+A, which the whole page takes:
/// `ui::select_all`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ListKey {
    /// Space, as [`space_action`] decides.
    Space,
}

/// The card list's keys: a key, whether it is taken only with Ctrl (`true`)
/// or only without it (`false`), and what it is. Other modifiers do not
/// matter. The Keyboard Shortcuts dialog lists them (a test checks it).
pub const LIST_KEYS: [(gtk::gdk::Key, bool, ListKey); 1] =
    [(gtk::gdk::Key::space, false, ListKey::Space)];

/// Which of [`LIST_KEYS`] `key` with `modifiers` is, if any.
pub fn list_key(key: gtk::gdk::Key, modifiers: gtk::gdk::ModifierType) -> Option<ListKey> {
    let ctrl = modifiers.contains(gtk::gdk::ModifierType::CONTROL_MASK);
    LIST_KEYS
        .iter()
        .find(|&&(bound, needs_ctrl, _)| bound == key && needs_ctrl == ctrl)
        .map(|&(_, _, which)| which)
}

/// What Space does on the card list (capture phase, before the list's own
/// keys).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SpaceAction {
    /// Choose or drop the focused card.
    Toggle,
    /// Leave it to the focused widget (a row's menu button opens its menu;
    /// a row outside selection mode does nothing). Space never starts a
    /// search: the page's key capture leaves it (`types_into_search`), as
    /// GTK's own does (gtksearchentry.c, `capture_widget_key_handled`).
    Pass,
}

/// Space chooses the focused card in selection mode when the row itself
/// (not its menu button) has the focus.
pub fn space_action(selecting: bool, row_focused: bool) -> SpaceAction {
    if selecting && row_focused {
        SpaceAction::Toggle
    } else {
        SpaceAction::Pass
    }
}

/// The note after an action found that a card of the list was gone.
pub fn stale_text() -> String {
    gettext("The list was out of date; nothing was changed")
}

/// The Suspend button's word (also its tooltip, like the other action
/// buttons') and accessible label for `choice`.
pub fn suspend_labels(choice: SuspendChoice) -> (String, String) {
    match choice {
        SuspendChoice::Suspend => (gettext("Suspend"), gettext("Suspend selected cards")),
        SuspendChoice::Unsuspend => (gettext("Unsuspend"), gettext("Unsuspend selected cards")),
    }
}

/// The body of the Move dialog.
pub fn move_body(n: usize) -> String {
    counted(
        // Translators: {count} is the number of cards chosen.
        &ngettext(
            "Choose the deck to move {count} card to.",
            "Choose the deck to move {count} cards to.",
            n,
        ),
        n,
    )
}

/// The body of the Add Tags dialog: what it does, and how a tag is ended
/// (the card editor's Tags group says the same).
pub fn add_tags_body(n: usize) -> String {
    counted(
        // Translators: {count} is the number of cards chosen.
        &ngettext(
            "Choose the tags to put on {count} card. Press Enter or type a comma after each one.",
            "Choose the tags to put on {count} cards. Press Enter or type a comma after each one.",
            n,
        ),
        n,
    )
}

/// The body of the Remove Tag dialog.
pub fn remove_tag_body(n: usize) -> String {
    counted(
        // Translators: {count} is the number of cards chosen.
        &ngettext(
            "Choose the tag to take off {count} card.",
            "Choose the tag to take off {count} cards.",
            n,
        ),
        n,
    )
}

/// The notice when the chosen cards have no tag to take off.
pub fn no_tags_text() -> String {
    gettext("The chosen cards have no tags")
}

pub fn moved_text(n: usize) -> String {
    // The deck was just chosen; short, so it fits beside Undo (ADR 0058).
    // Translators: an Undo toast, at most 24 characters with 99,999 cards
    // (it must fit beside Undo on a phone); {count} is a number of cards.
    counted(&ngettext("{count} card moved", "{count} cards moved", n), n)
}

/// The Undo toast after Add Tags put tags on `n` cards. Short, so it fits
/// beside Undo at 360 px (ADR 0058): the names were just typed.
pub fn tags_added_text(n: usize) -> String {
    // Translators: an Undo toast, at most 24 characters with 99,999 cards
    // (it must fit beside Undo on a phone); {count} is a number of cards.
    counted(
        &ngettext("{count} card tagged", "{count} cards tagged", n),
        n,
    )
}

/// The note after Add Tags left out `full` cards that would have had more
/// than `limit` tags, or `None`.
pub fn tags_left_out_text(full: usize, limit: usize) -> Option<String> {
    (full > 0).then(|| {
        fill(
            // Translators: a toast: its first sentence is its title, the
            // rest is behind Details. {count} is a number of cards,
            // {limit} the most tags a card can have (100).
            &ngettext(
                "{count} card left out. A card can have at most {limit} tags",
                "{count} cards left out. A card can have at most {limit} tags",
                full,
            ),
            &[
                ("count", &group_digits(full)),
                ("limit", &group_digits(limit)),
            ],
        )
    })
}

/// The notice when Add Tags changed no card because each would have had
/// more than `limit` tags.
pub fn no_room_text(limit: usize) -> String {
    fill(
        // Translators: a toast: what is before ";" is its title, the rest
        // is behind Details. {limit} is the most tags a card can have (100).
        &gettext("No tags added; a card can have at most {limit} tags"),
        &[("limit", &group_digits(limit))],
    )
}

pub fn tag_removed_text(n: usize) -> String {
    // The tag was just chosen; short, so it fits beside Undo (ADR 0058).
    // Translators: an Undo toast, at most 24 characters with 99,999 cards
    // (it must fit beside Undo on a phone); {count} is a number of cards.
    counted(
        &ngettext("{count} card untagged", "{count} cards untagged", n),
        n,
    )
}

pub fn suspended_text(n: usize, suspended: bool) -> String {
    if suspended {
        // Translators: an Undo toast, at most 24 characters with 99,999
        // cards (it must fit beside Undo on a phone).
        counted(
            &ngettext("{count} card suspended", "{count} cards suspended", n),
            n,
        )
    } else {
        // Translators: an Undo toast, at most 24 characters with 99,999
        // cards (it must fit beside Undo on a phone).
        counted(
            &ngettext("{count} card unsuspended", "{count} cards unsuspended", n),
            n,
        )
    }
}

pub fn reset_text(n: usize) -> String {
    // Short, so it fits beside Undo at 360 px (ADR 0058).
    // Translators: an Undo toast after Reset Progress, at most 24
    // characters with 99,999 cards (it must fit beside Undo on a phone).
    counted(&ngettext("{count} card reset", "{count} cards reset", n), n)
}

pub fn deleted_text(n: usize) -> String {
    // Translators: an Undo toast, at most 24 characters with 99,999 cards
    // (it must fit beside Undo on a phone).
    counted(
        &ngettext("{count} card deleted", "{count} cards deleted", n),
        n,
    )
}

/// What an action did when every card was already as asked.
pub fn unchanged_text() -> String {
    gettext("Nothing changed; the cards were already like that")
}

/// The note after an Undo that could not change every card (`done` of
/// `total`), or `None` if it did.
pub fn partial_undo_text(done: usize, total: usize) -> Option<String> {
    (done < total).then(|| {
        fill(
            // Translators: a toast: what is before ";" is its title, the
            // rest is behind Details. {done} of {count} cards came back.
            &ngettext(
                "{done} of {count} card restored; the others changed since",
                "{done} of {count} cards restored; the others changed since",
                total,
            ),
            &[
                ("done", &group_digits(done)),
                ("count", &group_digits(total)),
            ],
        )
    })
}

/// What Escape does on a page that lists cards (All Cards, a deck's
/// page), shared so both leave selection mode the same way.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EscapeAction {
    LeaveSelection,
    ClearSearch,
    Nothing,
}

/// Escape first leaves the selection mode (the mode you are in), then
/// clears the search and filters (All Cards; a deck's page has none, so
/// `filtered` is `false` there). In All Cards' search entry, the entry's
/// own `stop-search` clears the text first.
pub fn escape_action(selecting: bool, filtered: bool) -> EscapeAction {
    if selecting {
        EscapeAction::LeaveSelection
    } else if filtered {
        EscapeAction::ClearSearch
    } else {
        EscapeAction::Nothing
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn escape_leaves_the_selection_before_it_clears_the_search() {
        assert_eq!(escape_action(true, true), EscapeAction::LeaveSelection);
        assert_eq!(escape_action(false, true), EscapeAction::ClearSearch);
        assert_eq!(escape_action(false, false), EscapeAction::Nothing);
    }

    #[test]
    fn the_title_counts_the_chosen_cards() {
        assert_eq!(selection_title(0), "Select Cards");
        assert_eq!(selection_title(3), "3 Selected");
        assert_eq!(selection_title(1_200), "1,200 Selected");
        assert_eq!(selection_title_tight(0), "Select Cards");
        assert_eq!(selection_title_tight(3), "3");
        assert_eq!(selection_title_tight(1_200), "1,200");
    }

    #[test]
    fn a_row_says_whether_it_is_chosen_only_while_choosing() {
        assert_eq!(with_selected("2 + 2".to_owned(), None), "2 + 2");
        assert_eq!(
            with_selected("2 + 2".to_owned(), Some(true)),
            "2 + 2, selected"
        );
        assert_eq!(
            with_selected("2 + 2".to_owned(), Some(false)),
            "2 + 2, not selected"
        );
    }

    #[test]
    fn the_list_keys() {
        use gtk::gdk::{Key, ModifierType};
        let ctrl = ModifierType::CONTROL_MASK;
        let none = ModifierType::empty();
        // Ctrl+A is the page's (`ui::select_all`), not the list's.
        assert_eq!(list_key(Key::a, ctrl), None);
        assert_eq!(list_key(Key::a, none), None);
        assert_eq!(list_key(Key::space, none), Some(ListKey::Space));
        assert_eq!(
            list_key(Key::space, ModifierType::SHIFT_MASK),
            Some(ListKey::Space)
        );
        assert_eq!(list_key(Key::space, ctrl), None);
        assert_eq!(list_key(Key::Return, none), None);
    }

    #[test]
    fn unsuspend_is_offered_only_when_every_card_is_suspended() {
        use SuspendChoice::{Suspend, Unsuspend};
        assert_eq!(suspend_choice([true, true]), Unsuspend);
        assert_eq!(suspend_choice([true, false]), Suspend);
        assert_eq!(suspend_choice([false]), Suspend);
        assert_eq!(suspend_choice([]), Suspend);
    }

    #[test]
    fn reset_is_offered_while_a_chosen_card_has_progress() {
        assert!(reset_offered([true, false]));
        assert!(reset_offered([false]));
        assert!(!reset_offered([true, true]));
        assert!(!reset_offered([]));
        assert_eq!(no_progress_text(1), "This card has no progress to reset");
        assert_eq!(no_progress_text(3), "These cards have no progress to reset");
    }

    #[test]
    fn space_chooses_only_a_focused_row_in_selection_mode() {
        assert_eq!(space_action(true, true), SpaceAction::Toggle);
        assert_eq!(space_action(true, false), SpaceAction::Pass);
        assert_eq!(space_action(false, true), SpaceAction::Pass);
        assert_eq!(space_action(false, false), SpaceAction::Pass);
    }

    #[test]
    fn dialog_and_button_texts() {
        assert_eq!(move_body(3), "Choose the deck to move 3 cards to.");
        assert_eq!(remove_tag_body(1), "Choose the tag to take off 1 card.");
        assert_eq!(
            add_tags_body(3),
            "Choose the tags to put on 3 cards. Press Enter or type a comma after each one."
        );
        assert_eq!(
            suspend_labels(SuspendChoice::Unsuspend),
            (
                "Unsuspend".to_owned(),
                "Unsuspend selected cards".to_owned()
            )
        );
    }

    #[test]
    fn toasts_count_and_name() {
        assert_eq!(moved_text(3), "3 cards moved");
        assert_eq!(tags_added_text(2), "2 cards tagged");
        assert_eq!(tags_left_out_text(0, 100), None);
        assert_eq!(
            tags_left_out_text(1, 100).as_deref(),
            Some("1 card left out. A card can have at most 100 tags")
        );
        assert_eq!(
            tags_left_out_text(2_000, 1_000).as_deref(),
            Some("2,000 cards left out. A card can have at most 1,000 tags")
        );
        assert_eq!(
            no_room_text(100),
            "No tags added; a card can have at most 100 tags"
        );
        assert_eq!(tag_removed_text(1), "1 card untagged");
        assert_eq!(suspended_text(2, true), "2 cards suspended");
        assert_eq!(suspended_text(1, false), "1 card unsuspended");
        assert_eq!(reset_text(1_000), "1,000 cards reset");
        assert_eq!(deleted_text(1), "1 card deleted");
        // Each Undo toast's title fits beside Undo at 360 px (ADR 0058),
        // up to 99,999 cards, in English and in French.
        undo_titles_fit();
        crate::i18n::tests::in_french(undo_titles_fit);
        assert_eq!(
            unchanged_text(),
            "Nothing changed; the cards were already like that"
        );
    }

    #[test]
    fn a_partial_undo_says_how_much_came_back() {
        assert_eq!(partial_undo_text(3, 3), None);
        assert_eq!(
            partial_undo_text(2, 3).as_deref(),
            Some("2 of 3 cards restored; the others changed since")
        );
        assert_eq!(
            partial_undo_text(0, 1).as_deref(),
            Some("0 of 1 card restored; the others changed since")
        );
    }

    fn undo_titles_fit() {
        for n in [1, 2, 99_999] {
            for title in [
                moved_text(n),
                tags_added_text(n),
                tag_removed_text(n),
                suspended_text(n, false),
                suspended_text(n, true),
                reset_text(n),
                deleted_text(n),
            ] {
                assert!(
                    title.chars().count() <= crate::ui::toast::TOAST_FITS_WITH_BUTTON,
                    "{title}"
                );
            }
        }
        // The notes with Details keep their own titles.
        for text in [
            tags_left_out_text(99_999, 100).unwrap_or_default(),
            no_room_text(100),
            unchanged_text(),
            stale_text(),
        ] {
            if let crate::ui::toast::ToastText::Detailed { title, .. } =
                crate::ui::toast::toast_text(&text)
            {
                assert_ne!(title, crate::ui::toast::fallback_title(), "{text}");
            }
        }
    }
}
