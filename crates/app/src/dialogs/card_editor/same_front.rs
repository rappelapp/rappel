//! The note under the front when other cards have the same front (#119,
//! ADR 0072): what it says, decided without GTK.

use rappel_core::storage::SameFront;

use crate::i18n::{self, gettext, ngettext};
use crate::ui::text::group_digits;

/// The note for the cards `same` found, or `None` when there are none.
/// It names the deck when every card is in one deck, and says when that
/// deck, or one of several, is archived. Plain text: deck names are the
/// user's, so the label shows it without markup.
pub(super) fn note(same: &[SameFront]) -> Option<String> {
    let first = same.first()?;
    let one_deck = same.iter().all(|s| s.deck_id == first.deck_id);
    let archived = same.iter().any(|s| s.archived);
    let count = same.len();
    let deck = first.deck_name.as_str();
    let text = match (count, one_deck, archived) {
        (1, _, false) => i18n::fill(
            // Translators: under the front in the card editor. {deck} is a
            // deck's name.
            &gettext("A card in “{deck}” has the same front"),
            &[("deck", deck)],
        ),
        (1, _, true) => i18n::fill(
            // Translators: under the front in the card editor. {deck} is a
            // deck's name.
            &gettext("A card in the archived deck “{deck}” has the same front"),
            &[("deck", deck)],
        ),
        (_, true, false) => i18n::fill(
            // Translators: under the front in the card editor, for two or
            // more cards. {deck} is a deck's name.
            &ngettext(
                "{count} card in “{deck}” has the same front",
                "{count} cards in “{deck}” have the same front",
                count,
            ),
            &[("count", &group_digits(count)), ("deck", deck)],
        ),
        (_, true, true) => i18n::fill(
            // Translators: under the front in the card editor, for two or
            // more cards. {deck} is a deck's name.
            &ngettext(
                "{count} card in the archived deck “{deck}” has the same front",
                "{count} cards in the archived deck “{deck}” have the same front",
                count,
            ),
            &[("count", &group_digits(count)), ("deck", deck)],
        ),
        (_, false, false) => i18n::fill(
            // Translators: under the front in the card editor, for two or
            // more cards in two or more decks.
            &ngettext(
                "{count} card in several decks has the same front",
                "{count} cards in several decks have the same front",
                count,
            ),
            &[("count", &group_digits(count))],
        ),
        (_, false, true) => i18n::fill(
            // Translators: under the front in the card editor, for two or
            // more cards in two or more decks, one of them at least
            // archived.
            &ngettext(
                "{count} card in several decks has the same front, archived decks included",
                "{count} cards in several decks have the same front, archived decks included",
                count,
            ),
            &[("count", &group_digits(count))],
        ),
    };
    Some(text)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn card(n: u32, deck: &str, archived: bool) -> SameFront {
        SameFront {
            card_id: format!("card-{n}"),
            deck_id: format!("deck-{deck}"),
            deck_name: deck.to_owned(),
            archived,
        }
    }

    /// `n` cards in the deck "Spanish", archived or not.
    fn in_spanish(n: u32, archived: bool) -> Vec<SameFront> {
        (0..n).map(|i| card(i, "Spanish", archived)).collect()
    }

    #[test]
    fn no_card_no_note() {
        assert_eq!(note(&[]), None);
    }

    #[test]
    fn one_card_names_its_deck() {
        assert_eq!(
            note(&in_spanish(1, false)).as_deref(),
            Some("A card in “Spanish” has the same front")
        );
        assert_eq!(
            note(&in_spanish(1, true)).as_deref(),
            Some("A card in the archived deck “Spanish” has the same front")
        );
    }

    #[test]
    fn several_cards_in_one_deck_name_it() {
        assert_eq!(
            note(&in_spanish(2, false)).as_deref(),
            Some("2 cards in “Spanish” have the same front")
        );
        assert_eq!(
            note(&in_spanish(1_200, true)).as_deref(),
            Some("1,200 cards in the archived deck “Spanish” have the same front")
        );
    }

    #[test]
    fn several_decks_are_counted_and_an_archived_one_is_said() {
        let mut same = in_spanish(2, false);
        same.push(card(9, "French", false));
        assert_eq!(
            note(&same).as_deref(),
            Some("3 cards in several decks have the same front")
        );
        same.push(card(10, "Old", true));
        assert_eq!(
            note(&same).as_deref(),
            Some("4 cards in several decks have the same front, archived decks included")
        );
    }

    /// Decks are told apart by their ids, not their names.
    #[test]
    fn decks_are_told_apart_by_id() {
        let mut other = card(1, "Spanish", false);
        other.deck_id = "deck-other".to_owned();
        let same = [card(0, "Spanish", false), other];
        assert_eq!(
            note(&same).as_deref(),
            Some("2 cards in several decks have the same front")
        );
    }

    /// A deck name is shown as it is: the label has no markup.
    #[test]
    fn a_deck_name_is_left_as_typed() {
        let same = [card(0, "a&b <i>", false)];
        assert_eq!(
            note(&same).as_deref(),
            Some("A card in “a&b <i>” has the same front")
        );
    }

    #[test]
    fn the_note_is_in_french() {
        crate::i18n::tests::in_french(|| {
            assert_eq!(
                note(&in_spanish(1, false)).as_deref(),
                Some("Une carte de «\u{a0}Spanish\u{a0}» a le même recto")
            );
            assert_eq!(
                note(&in_spanish(3, true)).as_deref(),
                Some("3 cartes du paquet archivé «\u{a0}Spanish\u{a0}» ont le même recto")
            );
            let mut same = in_spanish(1, false);
            same.push(card(9, "Old", true));
            assert_eq!(
                note(&same).as_deref(),
                Some("2 cartes de plusieurs paquets ont le même recto, paquets archivés compris")
            );
        });
    }
}
