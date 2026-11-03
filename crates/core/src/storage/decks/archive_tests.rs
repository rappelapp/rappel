//! Archiving a deck (ADR 0068): what leaves the deck list, study and the
//! global statistics, what stays, and Unarchive.

use super::*;
use crate::domain::Grade;
use crate::stats::DayGrid;
use crate::storage::{BrowseQuery, Scope};
use crate::test_support::{DAY_MS, T0, answer, dump};

/// "Spanish", with a reviewed card and a new one, and "Maths", with one
/// reviewed card.
struct Two {
    store: Store,
    spanish: String,
    maths: String,
    reviewed: String,
    new: String,
    other: String,
}

fn two() -> Two {
    let mut store = Store::in_memory();
    let spanish = store.create_deck_at("Spanish", T0).unwrap().id;
    let maths = store.create_deck_at("Maths", T0).unwrap().id;
    let reviewed = store.add_card_at(&spanish, "hola", "hello", T0).unwrap().id;
    let new = store.add_card_at(&spanish, "gato", "cat", T0).unwrap().id;
    let other = store.add_card_at(&maths, "2 + 2", "4", T0).unwrap().id;
    answer(&mut store, &reviewed, Grade::Pass, T0 + 1);
    answer(&mut store, &other, Grade::Pass, T0 + 2);
    Two {
        store,
        spanish,
        maths,
        reviewed,
        new,
        other,
    }
}

fn ids(decks: &[Deck]) -> Vec<&str> {
    decks.iter().map(|d| d.id.as_str()).collect()
}

fn listed(store: &Store) -> Vec<String> {
    store
        .list_decks(T0, T0)
        .unwrap()
        .into_iter()
        .map(|d| d.deck.id)
        .collect()
}

#[test]
fn an_archived_deck_leaves_the_deck_list_and_keeps_its_name_and_cards() {
    let Two {
        mut store,
        spanish,
        maths,
        new,
        ..
    } = two();
    let before = store.deck(&spanish).unwrap();
    assert_eq!(before.archived_at, None);
    assert!(!before.is_archived());

    let archived = store.archive_deck_at(&spanish, T0 + 50).unwrap();

    assert_eq!(archived.archived_at, Some(T0 + 50));
    // Nothing else of the deck changes, so an older export is still the
    // same deck (ADR 0068).
    assert_eq!(
        Deck {
            archived_at: None,
            ..archived.clone()
        },
        before
    );
    assert_eq!(store.deck(&spanish).unwrap(), archived);
    assert_eq!(listed(&store), std::slice::from_ref(&maths));
    assert_eq!(ids(&store.decks().unwrap()), [maths.as_str()]);
    // Sorted by name, archived ones with their flag.
    let all = store.decks_with_archived().unwrap();
    assert_eq!(ids(&all), [maths.as_str(), spanish.as_str()]);
    assert!(all[1].is_archived() && !all[0].is_archived());
    assert_eq!(
        store.archived_decks().unwrap(),
        [ArchivedDeck {
            deck: archived,
            card_count: 2,
        }]
    );
    assert_eq!(store.card_count(&spanish).unwrap(), 2);
    assert_eq!(store.list_cards(&spanish).unwrap().len(), 2);
    assert!(store.card(&new).is_ok());
}

/// The tags in use, with or without archived decks, and whether any card
/// is left, archived or not.
#[test]
fn tags_in_use_and_live_cards_follow_the_archive() {
    let Two {
        mut store,
        spanish,
        reviewed,
        other,
        ..
    } = two();
    store.add_tag(&reviewed, "verbs").unwrap();
    store.add_tag(&other, "sums").unwrap();
    let names =
        |tags: Vec<crate::domain::Tag>| tags.into_iter().map(|t| t.name).collect::<Vec<_>>();
    store.archive_deck_at(&spanish, T0 + 50).unwrap();
    assert_eq!(names(store.used_tags().unwrap()), ["sums", "verbs"]);
    assert_eq!(
        names(store.used_tags_listed(true).unwrap()),
        ["sums", "verbs"]
    );
    assert_eq!(names(store.used_tags_listed(false).unwrap()), ["sums"]);
    assert!(store.has_live_cards().unwrap());
    let mut empty = Store::in_memory();
    assert!(!empty.has_live_cards().unwrap());
    let deck = empty.create_deck_at("x", T0).unwrap().id;
    let card = empty.add_card_at(&deck, "q", "a", T0).unwrap().id;
    empty.archive_deck_at(&deck, T0).unwrap();
    assert!(
        empty.has_live_cards().unwrap(),
        "an archived deck's card counts"
    );
    empty.delete_card(&card).unwrap();
    assert!(!empty.has_live_cards().unwrap());
}

#[test]
fn archived_decks_are_sorted_by_name_and_count_live_cards_only() {
    let mut store = Store::in_memory();
    let b = store.create_deck_at("b", T0).unwrap().id;
    let a = store.create_deck_at("A", T0).unwrap().id;
    let empty = store.create_deck_at("c", T0).unwrap().id;
    let kept = store.add_card_at(&b, "x", "y", T0).unwrap().id;
    let gone = store.add_card_at(&b, "z", "y", T0).unwrap().id;
    store
        .set_suspended(std::slice::from_ref(&kept), true)
        .unwrap();
    store.delete_card(&gone).unwrap();
    for id in [&b, &a, &empty] {
        store.archive_deck_at(id, T0 + 5).unwrap();
    }
    let archived: Vec<(String, u32)> = store
        .archived_decks()
        .unwrap()
        .into_iter()
        .map(|d| (d.deck.id, d.card_count))
        .collect();
    assert_eq!(archived, [(a, 0), (b, 1), (empty, 0)]);
    assert_eq!(store.list_decks(T0, T0).unwrap(), []);
}

#[test]
fn unarchive_restores_everything_as_before() {
    let Two {
        mut store, spanish, ..
    } = two();
    let before = dump(&store);
    let listed_before = store.list_decks(T0 + DAY_MS, T0).unwrap();

    store.archive_deck_at(&spanish, T0 + 50).unwrap();
    assert_ne!(dump(&store), before);
    let deck = store.unarchive_deck(&spanish).unwrap();

    assert_eq!(deck.archived_at, None);
    assert_eq!(dump(&store), before);
    assert_eq!(store.list_decks(T0 + DAY_MS, T0).unwrap(), listed_before);
    assert_eq!(store.archived_decks().unwrap(), []);
}

#[test]
fn archiving_an_archived_deck_is_refused_and_keeps_its_stamp() {
    let Two {
        mut store, spanish, ..
    } = two();
    store.archive_deck_at(&spanish, T0 + 50).unwrap();
    let before = dump(&store);
    assert!(matches!(
        store.archive_deck_at(&spanish, T0 + 90),
        Err(Error::DeckArchived(id)) if id == spanish
    ));
    assert_eq!(dump(&store), before);
    assert_eq!(store.deck(&spanish).unwrap().archived_at, Some(T0 + 50));
}

#[test]
fn missing_or_deleted_decks_cannot_be_archived_or_unarchived() {
    let Two {
        mut store, spanish, ..
    } = two();
    store.delete_deck(&spanish).unwrap();
    let before = dump(&store);
    let missing = "01920000-0000-7000-8000-000000000099";
    for id in [spanish.as_str(), missing] {
        assert!(matches!(
            store.archive_deck(id),
            Err(Error::DeckNotFound(_))
        ));
        assert!(matches!(
            store.unarchive_deck(id),
            Err(Error::DeckNotFound(_))
        ));
    }
    assert_eq!(dump(&store), before);
}

#[test]
fn unarchiving_a_deck_that_is_not_archived_changes_nothing() {
    let Two {
        mut store, maths, ..
    } = two();
    let before = dump(&store);
    let deck = store.unarchive_deck(&maths).unwrap();
    assert_eq!(deck, store.deck(&maths).unwrap());
    assert_eq!(dump(&store), before);
}

/// A session cannot start on an archived deck; Unarchive gives it back
/// its cards to study.
#[test]
fn an_archived_deck_is_not_studied() {
    let Two {
        mut store, spanish, ..
    } = two();
    let now = T0 + 30 * DAY_MS;
    let input = store
        .study_input(&crate::scheduling::StudyScope::deck(&spanish), now, now)
        .unwrap();
    assert_eq!(
        input.reviews.len() + input.steps.len() + input.new.len(),
        2,
        "{input:?}"
    );

    store.archive_deck_at(&spanish, T0 + 50).unwrap();
    assert!(matches!(
        store.study_input(&crate::scheduling::StudyScope::deck(&spanish), now, now),
        Err(Error::DeckArchived(id)) if id == spanish
    ));

    store.unarchive_deck(&spanish).unwrap();
    assert_eq!(
        store
            .study_input(&crate::scheduling::StudyScope::deck(&spanish), now, now)
            .unwrap(),
        input
    );
}

/// The global forecast and retention leave an archived deck out; the
/// activity (history) keeps it; the deck's own statistics stay whole.
#[test]
fn global_statistics_leave_out_the_forecast_and_retention_of_archived_decks() {
    let Two {
        mut store,
        spanish,
        maths,
        ..
    } = two();
    let now = T0 + 30 * DAY_MS;
    let past = DayGrid::new((0..=40).map(|i| now - (40 - i) * DAY_MS).collect()).unwrap();
    let ahead = DayGrid::new((0..=14).map(|i| now + i * DAY_MS).collect()).unwrap();
    let stats = |store: &Store, scope| store.stats(scope, now, &past, &ahead).unwrap();
    let all = stats(&store, Scope::All);
    let own = stats(&store, Scope::Deck(&spanish));
    let rest = stats(&store, Scope::Deck(&maths));
    assert_eq!(all.scheduled_cards, 2);

    store.archive_deck_at(&spanish, T0 + 50).unwrap();

    let after = stats(&store, Scope::All);
    assert_eq!(after.scheduled_cards, rest.scheduled_cards);
    assert_eq!(after.forecast, rest.forecast);
    assert_eq!(after.overdue, rest.overdue);
    assert_eq!(after.expected_retention, rest.expected_retention);
    // History: both decks' reviews still count.
    assert_eq!(after.days, all.days);
    assert_eq!(after.pass_rate, all.pass_rate);
    assert_eq!(stats(&store, Scope::Deck(&spanish)), own);

    store.unarchive_deck(&spanish).unwrap();
    assert_eq!(stats(&store, Scope::All), all);
}

#[test]
fn all_cards_hides_archived_decks_unless_asked() {
    let Two {
        mut store,
        spanish,
        reviewed,
        new,
        other,
        ..
    } = two();
    store.archive_deck_at(&spanish, T0 + 50).unwrap();
    let found = |store: &Store, query: &BrowseQuery| -> Vec<String> {
        store
            .browse(query, T0)
            .unwrap()
            .into_iter()
            .map(|r| r.card.id)
            .collect()
    };
    assert_eq!(
        found(&store, &BrowseQuery::default()),
        std::slice::from_ref(&other)
    );
    let shown = BrowseQuery {
        include_archived: true,
        ..BrowseQuery::default()
    };
    assert_eq!(found(&store, &shown), [other, new.clone(), reviewed]);
    // Filtered to the archived deck: only when shown.
    let deck = |include_archived| BrowseQuery {
        deck_id: Some(spanish.clone()),
        include_archived,
        ..BrowseQuery::default()
    };
    assert_eq!(found(&store, &deck(false)), Vec::<String>::new());
    assert_eq!(found(&store, &deck(true)).len(), 2);
    // A card's own row is found whatever the filter.
    assert!(store.browse_row(&new, T0).unwrap().is_some());
}

#[test]
fn an_archived_decks_name_stays_taken_and_says_so() {
    let Two {
        mut store,
        spanish,
        maths,
        ..
    } = two();
    store.archive_deck_at(&spanish, T0 + 50).unwrap();
    let before = dump(&store);
    for name in ["Spanish", "  SPANISH "] {
        assert!(matches!(
            store.create_deck(name),
            Err(Error::ArchivedDeckName(n)) if n == name.trim()
        ));
        assert!(matches!(
            store.rename_deck(&maths, name),
            Err(Error::ArchivedDeckName(_))
        ));
    }
    // A live deck that is not archived: the plain duplicate.
    assert!(matches!(
        store.create_deck("maths"),
        Err(Error::DuplicateDeckName(_))
    ));
    assert_eq!(dump(&store), before);
    // Unarchived, it is an ordinary duplicate again.
    store.unarchive_deck(&spanish).unwrap();
    assert!(matches!(
        store.create_deck("Spanish"),
        Err(Error::DuplicateDeckName(_))
    ));
}

#[test]
fn undelete_refuses_a_name_an_archived_deck_took() {
    let mut store = Store::in_memory();
    let old = store.create_deck_at("Spanish", T0).unwrap().id;
    store.delete_deck(&old).unwrap();
    let new = store.create_deck_at("Spanish", T0).unwrap().id;
    store.archive_deck_at(&new, T0 + 5).unwrap();
    let before = dump(&store);
    assert!(matches!(
        store.undelete_deck(&old),
        Err(Error::ArchivedDeckName(n)) if n == "Spanish"
    ));
    assert_eq!(dump(&store), before);
}

#[test]
fn a_deleted_archived_deck_comes_back_archived() {
    let Two {
        mut store, spanish, ..
    } = two();
    store.archive_deck_at(&spanish, T0 + 50).unwrap();
    store.delete_deck(&spanish).unwrap();
    assert_eq!(store.archived_decks().unwrap(), []);
    let deck = store.undelete_deck(&spanish).unwrap();
    assert_eq!(deck.archived_at, Some(T0 + 50));
    assert_eq!(store.archived_decks().unwrap().len(), 1);
    assert_eq!(store.card_count(&spanish).unwrap(), 2);
}

#[test]
fn a_merge_into_an_archived_deck_is_refused() {
    let Two {
        mut store,
        spanish,
        maths,
        ..
    } = two();
    store.archive_deck_at(&spanish, T0 + 50).unwrap();
    let before = dump(&store);
    assert!(matches!(
        store.merge_deck(&maths, &spanish),
        Err(Error::DeckArchived(id)) if id == spanish
    ));
    assert_eq!(dump(&store), before);
    // An archived deck can still be merged into a live one in core (the
    // app does not offer it); its cards are then studied with the target.
    let merged = store.merge_deck(&spanish, &maths).unwrap();
    assert_eq!(merged.live, 2);
    assert_eq!(store.archived_decks().unwrap(), []);
    // Its Undo brings it back archived.
    let deck = store.unmerge_deck(&merged).unwrap();
    assert_eq!(deck.archived_at, Some(T0 + 50));
    assert_eq!(dump(&store), before);
}
