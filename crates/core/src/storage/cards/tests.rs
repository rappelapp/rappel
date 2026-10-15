use std::time::Duration;

use super::*;
use crate::domain::{CardState, Grade};

/// 2026-10-15T12:34:56.789Z
const T0: i64 = 1_792_067_696_789;
const MISSING: &str = "01920000-0000-7000-8000-00000000dead";

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
    store.edit_card_at(&card.id, "q2", "a2", T0 + 7).unwrap();
    let edited = store.card(&card.id).unwrap();
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
        .record_review_at(&card.id, Grade::Pass, Duration::ZERO, T0)
        .unwrap();

    store.delete_card_at(&card.id, T0 + 1).unwrap();

    assert_eq!(store.list_cards(&deck).unwrap(), [other]);
    assert_eq!(store.due_cards(&deck, i64::MAX).unwrap().len(), 0);
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
