//! Every text of archiving that needs no GTK (ADR 0068): the deck list's
//! "Archived Decks" row and empty state, the Archived Decks page's rows,
//! and the toasts. Pure, so it is unit-tested.

use crate::i18n::{fill, gettext, ngettext};
use crate::ui::text::{counted, group_digits};

/// The count beside "Archived Decks" at the end of the deck list: "2".
pub fn row_count(archived: usize) -> String {
    group_digits(archived)
}

/// What a screen reader adds to the "Archived Decks" row: "2 decks".
pub fn row_description(archived: usize) -> String {
    // Translators: read with the "Archived Decks" row of the deck list;
    // {count} is how many decks are archived.
    counted(
        &ngettext("{count} deck", "{count} decks", archived),
        archived,
    )
}

/// The deck list's empty state, title and description: there is no deck
/// yet, or every deck is archived (`archived` of them).
pub fn deck_list_empty(archived: usize) -> (String, String) {
    if archived == 0 {
        (
            gettext("No Decks Yet"),
            gettext("Create a deck to start adding cards"),
        )
    } else {
        (
            gettext("Every Deck Is Archived"),
            gettext("Create a deck, or unarchive one to study it again"),
        )
    }
}

/// Under an archived deck's name: how many cards it has.
pub fn card_count(cards: usize) -> String {
    if cards == 0 {
        gettext("No cards")
    } else {
        // Translators: {count} is a number of cards.
        counted(&ngettext("{count} card", "{count} cards", cards), cards)
    }
}

/// The button of an archived deck's row.
pub fn unarchive_label() -> String {
    gettext("Unarchive")
}

/// The tooltip and accessible label of an archived deck's Unarchive
/// button, naming the deck.
pub fn unarchive_tooltip(name: &str) -> String {
    fill(
        // Translators: the tooltip of an archived deck's Unarchive button;
        // {name} is the deck's name.
        &gettext("Unarchive “{name}”"),
        &[("name", name)],
    )
}

/// The Undo toast after Archive. Short, so it fits beside Undo at 360 px
/// (ADR 0058).
pub fn archived_toast() -> String {
    gettext("Deck archived")
}

/// The Undo toast after Unarchive.
pub fn unarchived_toast() -> String {
    gettext("Deck unarchived")
}

/// A toast's text when archiving failed (an Unarchive's Undo too).
pub fn not_archived(err: &impl std::fmt::Display) -> String {
    crate::ui::text::with_error(
        // Translators: a toast: what is before ":" is its title, the rest is behind Details. {error} is why, in English.
        &gettext("The deck could not be archived: {error}"),
        err,
    )
}

/// A toast's text when unarchiving failed (an Archive's Undo too).
pub fn not_unarchived(err: &impl std::fmt::Display) -> String {
    crate::ui::text::with_error(
        // Translators: a toast: what is before ":" is its title, the rest is behind Details. {error} is why, in English.
        &gettext("The deck could not be unarchived: {error}"),
        err,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::toast::TOAST_FITS_WITH_BUTTON;

    #[test]
    fn the_row_counts_the_archived_decks() {
        assert_eq!(row_count(2), "2");
        assert_eq!(row_count(1_200), "1,200");
        assert_eq!(row_description(1), "1 deck");
        assert_eq!(row_description(3), "3 decks");
    }

    #[test]
    fn the_empty_deck_list_says_when_every_deck_is_archived() {
        assert_eq!(
            deck_list_empty(0),
            (
                "No Decks Yet".to_owned(),
                "Create a deck to start adding cards".to_owned()
            )
        );
        let (title, description) = deck_list_empty(2);
        assert_eq!(title, "Every Deck Is Archived");
        assert_eq!(
            description,
            "Create a deck, or unarchive one to study it again"
        );
    }

    #[test]
    fn an_archived_row_says_its_cards_and_names_its_deck() {
        assert_eq!(card_count(0), "No cards");
        assert_eq!(card_count(1), "1 card");
        assert_eq!(card_count(5_210), "5,210 cards");
        assert_eq!(unarchive_label(), "Unarchive");
        assert_eq!(unarchive_tooltip("Spanish"), "Unarchive “Spanish”");
        // Braces in a name stay as they are.
        assert_eq!(unarchive_tooltip("{name}"), "Unarchive “{name}”");
    }

    #[test]
    fn the_toasts_fit() {
        for text in [archived_toast(), unarchived_toast()] {
            assert!(text.chars().count() <= TOAST_FITS_WITH_BUTTON, "{text}");
        }
        let err = rappel_core::Error::DeckNotFound("x".to_owned());
        assert!(not_archived(&err).starts_with("The deck could not be archived: "));
        assert!(not_unarchived(&err).starts_with("The deck could not be unarchived: "));
    }
}
