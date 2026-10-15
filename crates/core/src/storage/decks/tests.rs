use std::time::Duration;

use super::*;
use crate::domain::Grade;

/// 2026-10-15T12:34:56.789Z
const T0: i64 = 1_792_067_696_789;
const DAY_MS: i64 = 86_400_000;

fn deleted_at(store: &Store, table: &str, id: &str) -> Option<i64> {
    store
        .conn
        .query_row(
            &format!("SELECT deleted_at FROM {table} WHERE id = ?1"),
            [id],
            |r| r.get(0),
        )
        .unwrap()
}

fn counts(store: &Store, now: i64) -> Vec<(String, u32, u32)> {
    store
        .list_decks(now)
        .unwrap()
        .into_iter()
        .map(|d| (d.deck.id, d.new_count, d.due_count))
        .collect()
}

fn names(store: &Store) -> Vec<String> {
    store
        .list_decks(T0)
        .unwrap()
        .into_iter()
        .map(|d| d.deck.name)
        .collect()
}

#[test]
fn create_trims_the_name_and_lists_the_deck() {
    let mut store = Store::in_memory();
    let deck = store.create_deck_at("  Maths \n", T0).unwrap();
    assert_eq!(deck.name, "Maths");
    assert_eq!((deck.created_at, deck.updated_at), (T0, T0));
    assert_eq!(deck.id.len(), 36);
    assert_eq!(
        store.list_decks(T0).unwrap(),
        [DeckSummary {
            deck,
            new_count: 0,
            due_count: 0
        }]
    );
}

#[test]
fn empty_names_are_refused() {
    let mut store = Store::in_memory();
    for name in ["", "   ", "\t\n"] {
        assert!(matches!(
            store.create_deck_at(name, T0),
            Err(Error::EmptyDeckName)
        ));
    }
    let deck = store.create_deck_at("Maths", T0).unwrap();
    assert!(matches!(
        store.rename_deck_at(&deck.id, " ", T0),
        Err(Error::EmptyDeckName)
    ));
    assert_eq!(names(&store), ["Maths"]);
}

#[test]
fn rename_changes_name_and_updated_at() {
    let mut store = Store::in_memory();
    let deck = store.create_deck_at("Maths", T0).unwrap();
    store.rename_deck_at(&deck.id, " Algebra ", T0 + 5).unwrap();
    let listed = store.list_decks(T0).unwrap().remove(0).deck;
    assert_eq!(listed.name, "Algebra");
    assert_eq!((listed.created_at, listed.updated_at), (T0, T0 + 5));
}

#[test]
fn rename_to_the_same_name_changes_nothing() {
    let mut store = Store::in_memory();
    let deck = store.create_deck_at("Maths", T0).unwrap();
    store.rename_deck_at(&deck.id, "Maths ", T0 + 5).unwrap();
    assert_eq!(store.list_decks(T0).unwrap()[0].deck, deck);
}

#[test]
fn missing_or_deleted_decks_cannot_be_renamed_or_deleted() {
    let mut store = Store::in_memory();
    let missing = "01920000-0000-7000-8000-00000000dead";
    assert!(
        matches!(store.rename_deck_at(missing, "X", T0), Err(Error::DeckNotFound(id)) if id == missing)
    );
    assert!(matches!(
        store.delete_deck_at(missing, T0),
        Err(Error::DeckNotFound(_))
    ));

    let deck = store.create_deck_at("Maths", T0).unwrap();
    store.delete_deck_at(&deck.id, T0).unwrap();
    assert!(matches!(
        store.rename_deck_at(&deck.id, "X", T0),
        Err(Error::DeckNotFound(_))
    ));
    // Renaming to its own name does not reveal a deleted deck either.
    assert!(matches!(
        store.rename_deck_at(&deck.id, "Maths", T0),
        Err(Error::DeckNotFound(_))
    ));
    assert!(matches!(
        store.delete_deck_at(&deck.id, T0),
        Err(Error::DeckNotFound(_))
    ));
}

#[test]
fn duplicate_names_are_refused_on_create_and_rename() {
    let mut store = Store::in_memory();
    let maths = store.create_deck_at("Maths", T0).unwrap();
    assert!(matches!(
        store.create_deck_at(" Maths ", T0),
        Err(Error::DuplicateDeckName(n)) if n == "Maths"
    ));
    let physics = store.create_deck_at("Physics", T0).unwrap();
    assert!(matches!(
        store.rename_deck_at(&physics.id, "Maths", T0),
        Err(Error::DuplicateDeckName(_))
    ));
    // Exact comparison: a different case is a different name.
    store.create_deck_at("maths", T0).unwrap();
    assert_eq!(names(&store), ["Maths", "maths", "Physics"]);
    assert_eq!(store.list_decks(T0).unwrap()[0].deck, maths);
}

#[test]
fn a_deleted_decks_name_can_be_reused() {
    let mut store = Store::in_memory();
    let old = store.create_deck_at("Maths", T0).unwrap();
    store.delete_deck_at(&old.id, T0).unwrap();
    let new = store.create_deck_at("Maths", T0).unwrap();
    assert_ne!(old.id, new.id);

    // And by rename, after the new one is deleted too.
    store.delete_deck_at(&new.id, T0).unwrap();
    let other = store.create_deck_at("Physics", T0).unwrap();
    store.rename_deck_at(&other.id, "Maths", T0).unwrap();
    assert_eq!(names(&store), ["Maths"]);
}

#[test]
fn list_is_sorted_by_name_ignoring_case() {
    let mut store = Store::in_memory();
    for name in ["physics", "Algebra", "Écologie", "biology"] {
        store.create_deck_at(name, T0).unwrap();
    }
    // Lower-casing, not locale collation: "é" sorts after ASCII letters.
    assert_eq!(names(&store), ["Algebra", "biology", "physics", "Écologie"]);
}

#[test]
fn counts_new_and_due_live_cards() {
    let mut store = Store::in_memory();
    let maths = store.create_deck_at("Maths", T0).unwrap();
    let empty = store.create_deck_at("Empty", T0).unwrap();
    let cards: Vec<_> = (0..5)
        .map(|i| {
            store
                .add_card_at(&maths.id, &format!("q{i}"), "a", T0)
                .unwrap()
                .id
        })
        .collect();
    // cards[0], cards[1] reviewed: due in 10 minutes. cards[2] deleted.
    for c in &cards[..2] {
        store
            .record_review_at(c, Grade::Fail, Duration::ZERO, T0)
            .unwrap();
    }
    store.delete_card_at(&cards[2], T0).unwrap();

    assert_eq!(
        counts(&store, T0),
        [(empty.id.clone(), 0, 0), (maths.id.clone(), 2, 0)]
    );
    assert_eq!(
        counts(&store, T0 + DAY_MS),
        [(empty.id.clone(), 0, 0), (maths.id.clone(), 2, 2)]
    );

    // A deleted reviewed card is not due.
    store.delete_card_at(&cards[0], T0).unwrap();
    assert_eq!(
        counts(&store, T0 + DAY_MS),
        [(empty.id, 0, 0), (maths.id, 2, 1)]
    );
}

#[test]
fn deleting_a_deck_hides_it_and_its_cards_and_keeps_their_history() {
    let mut store = Store::in_memory();
    let maths = store.create_deck_at("Maths", T0).unwrap();
    let other = store.create_deck_at("Other", T0).unwrap();
    let card = store.add_card_at(&maths.id, "q", "a", T0).unwrap();
    let kept = store.add_card_at(&other.id, "q", "a", T0).unwrap();
    store
        .record_review_at(&card.id, Grade::Pass, Duration::from_secs(3), T0)
        .unwrap();

    store.delete_deck_at(&maths.id, T0 + 1).unwrap();

    assert_eq!(names(&store), ["Other"]);
    assert!(matches!(
        store.list_cards(&maths.id),
        Err(Error::DeckNotFound(_))
    ));
    assert!(matches!(
        store.due_cards(&maths.id, T0),
        Err(Error::DeckNotFound(_))
    ));
    assert!(matches!(
        store.new_cards(&maths.id, 10),
        Err(Error::DeckNotFound(_))
    ));
    assert!(matches!(store.card(&card.id), Err(Error::CardNotFound(_))));
    assert!(matches!(
        store.add_card_at(&maths.id, "q", "a", T0),
        Err(Error::DeckNotFound(_))
    ));
    assert!(matches!(
        store.record_review_at(&card.id, Grade::Pass, Duration::ZERO, T0 + 2),
        Err(Error::CardNotFound(_))
    ));
    // History kept; the other deck untouched.
    assert_eq!(store.review_events(&card.id).unwrap().len(), 1);
    assert_eq!(store.card(&kept.id).unwrap(), kept);
}

#[test]
fn deck_delete_stamps_the_deck_and_its_live_cards_with_one_value() {
    let mut store = Store::in_memory();
    let maths = store.create_deck_at("Maths", T0).unwrap();
    let other = store.create_deck_at("Other", T0).unwrap();
    let a = store.add_card_at(&maths.id, "a", "", T0).unwrap();
    let b = store.add_card_at(&maths.id, "b", "", T0).unwrap();
    let earlier = store.add_card_at(&maths.id, "c", "", T0).unwrap();
    let elsewhere = store.add_card_at(&other.id, "d", "", T0).unwrap();
    store.delete_card_at(&earlier.id, T0 + 10).unwrap();

    store.delete_deck_at(&maths.id, T0 + 20).unwrap();

    assert_eq!(deleted_at(&store, "decks", &maths.id), Some(T0 + 20));
    assert_eq!(deleted_at(&store, "cards", &a.id), Some(T0 + 20));
    assert_eq!(deleted_at(&store, "cards", &b.id), Some(T0 + 20));
    // Deleted before: keeps its own stamp, so it is not part of the batch.
    assert_eq!(deleted_at(&store, "cards", &earlier.id), Some(T0 + 10));
    assert_eq!(deleted_at(&store, "cards", &elsewhere.id), None);
    assert_eq!(deleted_at(&store, "decks", &other.id), None);
}

#[test]
fn deck_delete_stamp_is_after_earlier_card_deletes_in_the_same_millisecond() {
    let mut store = Store::in_memory();
    let maths = store.create_deck_at("Maths", T0).unwrap();
    let card = store.add_card_at(&maths.id, "a", "", T0).unwrap();
    let earlier = store.add_card_at(&maths.id, "b", "", T0).unwrap();
    // Same millisecond, and a clock that went backwards: the batch stamp
    // must still differ from the earlier single delete.
    store.delete_card_at(&earlier.id, T0 + 10).unwrap();
    store.delete_deck_at(&maths.id, T0 + 5).unwrap();

    let stamp = deleted_at(&store, "decks", &maths.id).unwrap();
    assert_eq!(stamp, T0 + 11);
    assert_eq!(deleted_at(&store, "cards", &card.id), Some(stamp));
    assert_eq!(deleted_at(&store, "cards", &earlier.id), Some(T0 + 10));
}
