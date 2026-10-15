use std::time::Duration;

use super::*;
use crate::domain::{CardState, Grade};

/// 2026-10-15T12:34:56.789Z
const T0: i64 = 1_792_067_696_789;
const MISSING: &str = "01920000-0000-7000-8000-00000000dead";

/// How many reviewed live cards of `deck` are due at any time: reviews
/// and step cards in [`Store::study_input`].
fn due_count(store: &Store, deck: &str) -> usize {
    let input = store.study_input(deck, i64::MAX, 0).unwrap();
    input.reviews.len() + input.steps.len()
}

fn store_with_deck() -> (Store, String) {
    let mut store = Store::in_memory();
    let deck = store.create_deck_at("Maths", T0).unwrap().id;
    (store, deck)
}

#[test]
fn add_stores_markdown_as_given_and_lists_in_creation_order() {
    let (mut store, deck) = store_with_deck();
    let a = store
        .add_card_at(&deck, "  $x^2$ <b>?</b>\n", "", T0)
        .unwrap();
    let b = store.add_card_at(&deck, "second", "back", T0).unwrap();
    assert_eq!(a.front_md, "  $x^2$ <b>?</b>\n");
    assert_eq!(
        (a.deck_id.as_str(), a.created_at, a.updated_at),
        (deck.as_str(), T0, T0)
    );
    assert!(a.id < b.id, "ids increase within a millisecond");
    assert_eq!(store.list_cards(&deck).unwrap(), [a.clone(), b]);
    assert_eq!(store.card(&a.id).unwrap(), a);
}

#[test]
fn blank_fronts_are_refused() {
    let (mut store, deck) = store_with_deck();
    for front in ["", " \n\t"] {
        assert!(matches!(
            store.add_card_at(&deck, front, "back", T0),
            Err(Error::EmptyCardFront)
        ));
    }
    let card = store.add_card_at(&deck, "q", "a", T0).unwrap();
    assert!(matches!(
        store.edit_card_at(&card.id, "  ", "a", T0),
        Err(Error::EmptyCardFront)
    ));
    assert_eq!(store.list_cards(&deck).unwrap(), [card]);
}

#[test]
fn add_needs_a_live_deck() {
    let (mut store, _) = store_with_deck();
    assert!(matches!(
        store.add_card_at(MISSING, "q", "a", T0),
        Err(Error::DeckNotFound(id)) if id == MISSING
    ));
    assert!(matches!(
        store.list_cards(MISSING),
        Err(Error::DeckNotFound(_))
    ));
}

#[test]
fn edit_replaces_content_and_bumps_updated_at() {
    let (mut store, deck) = store_with_deck();
    let card = store.add_card_at(&deck, "q", "a", T0).unwrap();
    let returned = store.edit_card_at(&card.id, "q2", "a2", T0 + 7).unwrap();
    let edited = store.card(&card.id).unwrap();
    assert_eq!(returned, edited);
    assert_eq!(
        edited,
        Card {
            front_md: "q2".into(),
            back_md: "a2".into(),
            updated_at: T0 + 7,
            ..card
        }
    );
}

#[test]
fn deleted_cards_are_hidden_and_refuse_changes_but_keep_history() {
    let (mut store, deck) = store_with_deck();
    let card = store.add_card_at(&deck, "q", "a", T0).unwrap();
    let other = store.add_card_at(&deck, "q", "a", T0).unwrap();
    store
        .record_review(&card.id, Grade::Pass, Duration::ZERO, T0)
        .unwrap();

    store.delete_card_at(&card.id, T0 + 1).unwrap();

    assert_eq!(store.list_cards(&deck).unwrap(), [other]);
    assert_eq!(due_count(&store, &deck), 0);
    assert!(matches!(store.card(&card.id), Err(Error::CardNotFound(_))));
    assert!(matches!(
        store.edit_card_at(&card.id, "x", "y", T0),
        Err(Error::CardNotFound(_))
    ));
    assert!(matches!(
        store.delete_card_at(&card.id, T0),
        Err(Error::CardNotFound(_))
    ));
    assert_eq!(store.review_events(&card.id).unwrap().len(), 1);
    assert!(matches!(
        store.card_state(&card.id).unwrap(),
        CardState::Scheduled(_)
    ));
}

#[test]
fn missing_cards_are_reported() {
    let (mut store, _) = store_with_deck();
    assert!(matches!(store.card(MISSING), Err(Error::CardNotFound(id)) if id == MISSING));
    assert!(matches!(
        store.edit_card_at(MISSING, "q", "a", T0),
        Err(Error::CardNotFound(_))
    ));
    assert!(matches!(
        store.delete_card_at(MISSING, T0),
        Err(Error::CardNotFound(_))
    ));
}

#[test]
fn undelete_restores_the_card_in_place_with_its_history() {
    let (mut store, deck) = store_with_deck();
    let a = store.add_card_at(&deck, "a", "", T0).unwrap();
    let b = store.add_card_at(&deck, "b", "back", T0).unwrap();
    let c = store.add_card_at(&deck, "c", "", T0).unwrap();
    store
        .record_review(&b.id, Grade::Pass, Duration::ZERO, T0)
        .unwrap();
    let events = store.review_events(&b.id).unwrap();
    let state = store.card_state(&b.id).unwrap();
    let counts = |store: &Store| {
        let summary = &store.list_decks(i64::MAX).unwrap()[0];
        (summary.new_count, summary.due_count)
    };
    assert_eq!(counts(&store), (2, 1));

    store.delete_card_at(&b.id, T0 + 5).unwrap();
    assert_eq!(counts(&store), (2, 0));
    let restored = store.undelete_card(&b.id).unwrap();

    assert_eq!(restored, b, "content and timestamps are unchanged");
    assert_eq!(store.list_cards(&deck).unwrap(), [a, b.clone(), c]);
    assert_eq!(store.card(&b.id).unwrap(), b);
    assert_eq!(store.review_events(&b.id).unwrap(), events);
    assert_eq!(store.card_state(&b.id).unwrap(), state);
    assert_eq!(counts(&store), (2, 1));
    assert_eq!(due_count(&store, &deck), 1);
}

#[test]
fn undelete_of_a_live_card_changes_nothing() {
    let (mut store, deck) = store_with_deck();
    let card = store.add_card_at(&deck, "q", "a", T0).unwrap();
    assert_eq!(store.undelete_card(&card.id).unwrap(), card);
    store.delete_card_at(&card.id, T0 + 1).unwrap();
    store.undelete_card(&card.id).unwrap();
    // A second Undo click.
    assert_eq!(store.undelete_card(&card.id).unwrap(), card);
    assert_eq!(store.list_cards(&deck).unwrap(), [card]);
}

#[test]
fn undelete_of_an_unknown_card_is_reported() {
    let (mut store, _) = store_with_deck();
    assert!(matches!(
        store.undelete_card(MISSING),
        Err(Error::CardNotFound(id)) if id == MISSING
    ));
}

#[test]
fn undelete_refuses_a_card_of_a_deleted_deck_until_the_deck_is_back() {
    let (mut store, deck) = store_with_deck();
    let alone = store.add_card_at(&deck, "alone", "", T0).unwrap();
    let with_deck = store.add_card_at(&deck, "with deck", "", T0).unwrap();
    store.delete_card_at(&alone.id, T0 + 1).unwrap();
    store.delete_deck_at(&deck, T0 + 2).unwrap();

    assert!(matches!(
        store.undelete_card(&alone.id),
        Err(Error::DeckNotFound(id)) if id == deck
    ));
    // The refused undo changed nothing: undoing the deck brings back only
    // the cards deleted with it.
    store.undelete_deck(&deck).unwrap();
    assert_eq!(
        store.list_cards(&deck).unwrap(),
        std::slice::from_ref(&with_deck)
    );

    assert_eq!(store.undelete_card(&alone.id).unwrap(), alone);
    assert_eq!(store.list_cards(&deck).unwrap(), [alone, with_deck]);
}

#[test]
fn an_undeleted_card_goes_with_its_deck_again() {
    let (mut store, deck) = store_with_deck();
    let card = store.add_card_at(&deck, "q", "a", T0).unwrap();
    store.delete_card_at(&card.id, T0 + 1).unwrap();
    store.undelete_card(&card.id).unwrap();
    // Same millisecond as the card's own delete.
    store.delete_deck_at(&deck, T0 + 1).unwrap();
    store.undelete_deck(&deck).unwrap();
    assert_eq!(store.list_cards(&deck).unwrap(), [card]);
}

#[test]
fn thousands_of_cards_list_in_creation_order() {
    let (mut store, deck) = store_with_deck();
    let ids: Vec<String> = (0..5_000)
        .map(|i| {
            store
                .add_card_at(&deck, &format!("q{i}"), "", T0)
                .unwrap()
                .id
        })
        .collect();
    let listed: Vec<String> = store
        .list_cards(&deck)
        .unwrap()
        .into_iter()
        .map(|c| c.id)
        .collect();
    assert_eq!(listed, ids);
}
