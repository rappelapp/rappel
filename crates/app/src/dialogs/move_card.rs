//! Move to Deck… for one card (#102): the choice of a deck, shared by the
//! deck page's card menu, All Cards' row menu and the review page's Card
//! Menu, and the Undo toast they all offer. The dialog itself is All Cards'
//! chooser (`browser_dialogs::choose_closing`, layout in `data/browser_dialogs.blp`);
//! the move and its Undo are core's `move_cards` and `unmove_cards` through
//! `AppState`. What each page does with the card afterwards (the row leaves
//! the list, the card leaves the session) is its own.

use adw::prelude::*;
use rappel_core::domain::Deck;
use rappel_core::storage::Moved;

use super::browser_dialogs;
use crate::i18n::gettext;
use crate::state::AppState;
use crate::ui::toast::undo_toast;

/// The decks a card of deck `from` can be moved to: every deck but its
/// own and the archived ones (a card moved there would leave study
/// unseen, ADR 0068), in the order given.
pub fn targets<'a>(decks: &'a [Deck], from: &str) -> Vec<&'a Deck> {
    decks
        .iter()
        .filter(|deck| deck.id != from && !deck.is_archived())
        .collect()
}

/// The decks a card of deck `from` can be moved to from its menu, where
/// the card's deck is read from the list (All Cards): `None` when the card
/// is not listed (nothing to do), else every deck but its own (empty: no
/// other deck, so only a notice).
pub fn card_targets(decks: &[Deck], from: Option<&str>) -> Option<Vec<Deck>> {
    let from = from?;
    Some(targets(decks, from).into_iter().cloned().collect())
}

/// Whether Move to Deck… is offered. For a card of deck `from` it is when
/// another deck exists. Where the card's deck is not known in advance (a
/// list of cards of every deck), `from` is `None` and it is when there are
/// two decks, since a card then has another one whatever its own. With no
/// other deck the entry is disabled, as a dialog with nothing to choose
/// would be a dead end.
pub fn offered(decks: &[Deck], from: Option<&str>) -> bool {
    match from {
        Some(from) => !targets(decks, from).is_empty(),
        None => decks.iter().filter(|deck| !deck.is_archived()).count() >= 2,
    }
}

/// The chooser's heading.
pub fn heading() -> String {
    gettext("Move Card")
}

/// The chooser's text.
pub fn body() -> String {
    gettext("Choose the deck to move this card to.")
}

/// The Undo toast's title. Short, so it fits beside Undo at 360 px
/// (ADR 0058), like "Card suspended".
pub fn moved_text() -> String {
    gettext("Card moved")
}

/// What to tell when the card could not be moved back: it was moved again
/// or deleted since, or its first deck is gone.
pub fn not_moved_back_text() -> String {
    gettext("The card could not be moved back")
}

/// What to tell when there is no deck to move the card to (the last other
/// deck was deleted since the menu was shown).
pub fn no_other_deck_text() -> String {
    gettext("No other deck to move it to")
}

/// Asks which of `targets` to move a card to, over `parent`, and returns
/// the one chosen, or `None` if cancelled. Deck names are the user's own
/// and are shown as plain text. `on_closed` is called once the dialog has
/// closed, for the page to put the focus where the answer leaves it (see
/// `browser_dialogs::choose_closing`).
pub async fn choose_deck(
    parent: &impl IsA<gtk::Widget>,
    targets: Vec<Deck>,
    on_closed: impl Fn() + 'static,
) -> Option<Deck> {
    let names: Vec<String> = targets.iter().map(|deck| deck.name.clone()).collect();
    let chosen = browser_dialogs::choose_closing(
        parent,
        &heading(),
        &body(),
        &gettext("_Move"),
        &names,
        on_closed,
    )
    .await?;
    targets.into_iter().nth(chosen)
}

/// The toast "Card moved" with Undo. Undo moves the cards in `moved` back
/// and gives `after` the result (how many went back); it keeps the state,
/// not a page, so it works after the page was left.
pub fn moved_toast(
    state: &AppState,
    moved: Vec<Moved>,
    after: impl Fn(rappel_core::Result<usize>) + 'static,
) -> adw::Toast {
    let state = state.clone();
    undo_toast(&moved_text(), move || after(state.unmove_cards(&moved)))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn deck(id: &str) -> Deck {
        Deck {
            id: id.to_owned(),
            name: format!("Deck {id}"),
            created_at: 0,
            updated_at: 0,
            archived_at: None,
        }
    }

    #[test]
    fn the_cards_own_deck_is_not_offered() {
        let decks = [deck("a"), deck("b"), deck("c")];
        let ids = |from| -> Vec<&str> {
            targets(&decks, from)
                .into_iter()
                .map(|d| d.id.as_str())
                .collect()
        };
        assert_eq!(ids("a"), ["b", "c"]);
        assert_eq!(ids("b"), ["a", "c"]);
        // A deck that is not in the list (deleted meanwhile) removes nothing.
        assert_eq!(ids("z"), ["a", "b", "c"]);
    }

    #[test]
    fn a_cards_move_targets_leave_out_its_own_deck() {
        let decks = [deck("a"), deck("b")];
        let ids = |targets: Option<Vec<Deck>>| -> Option<Vec<String>> {
            targets.map(|t| t.into_iter().map(|d| d.id).collect())
        };
        assert_eq!(
            ids(card_targets(&decks, Some("a"))),
            Some(vec!["b".to_owned()])
        );
        // One deck: nothing to choose, so only a notice.
        assert_eq!(ids(card_targets(&decks[..1], Some("a"))), Some(vec![]));
        // A card that is not listed: nothing is done at all.
        assert_eq!(card_targets(&decks, None), None);
    }

    #[test]
    fn the_entry_is_offered_only_with_another_deck() {
        let one = [deck("a")];
        let two = [deck("a"), deck("b")];
        assert!(!offered(&[], Some("a")));
        assert!(!offered(&one, Some("a")));
        assert!(offered(&two, Some("a")));
        // A card whose deck is not in the list can go to any deck there.
        assert!(offered(&one, Some("z")));
        // Where the card's deck is not known: two decks are needed.
        assert!(!offered(&[], None));
        assert!(!offered(&one, None));
        assert!(offered(&two, None));
    }

    /// Archived decks are never offered (ADR 0068).
    #[test]
    fn archived_decks_are_not_targets() {
        let archived = Deck {
            archived_at: Some(1),
            ..deck("b")
        };
        let decks = [deck("a"), archived.clone(), deck("c")];
        let ids: Vec<&str> = targets(&decks, "a")
            .into_iter()
            .map(|d| d.id.as_str())
            .collect();
        assert_eq!(ids, ["c"]);
        assert!(!offered(&[deck("a"), archived.clone()], Some("a")));
        assert!(!offered(&[deck("a"), archived], None));
    }

    #[test]
    fn the_texts_fit_a_toast() {
        use crate::ui::toast::{TOAST_FITS, TOAST_FITS_WITH_BUTTON};
        assert!(moved_text().chars().count() <= TOAST_FITS_WITH_BUTTON);
        for text in [not_moved_back_text(), no_other_deck_text()] {
            assert!(text.chars().count() <= TOAST_FITS, "{text}");
        }
        assert!(!body().contains('{') && !heading().contains('{'));
    }
}
