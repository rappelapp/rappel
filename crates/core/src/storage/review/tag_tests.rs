//! Studying by tag (ADR 0069): the scope's decks, the limits shared with
//! the decks, and what is left out.

use std::collections::HashSet;

use proptest::prelude::*;

use crate::Error;
use crate::domain::Grade;
use crate::scheduling::session::{EARLY_MS, StudyInput};
use crate::scheduling::{STEP_MS, StudyScope};
use crate::storage::Store;
use crate::test_support::{DAY_MS, T0, answer, learn, tag_id};

/// Study time, 40 days after the cards were learned: those are due.
const NOW: i64 = T0 + 40 * DAY_MS;
/// "Today" began an hour before `NOW`.
const DAY_START: i64 = NOW - 3_600_000;

/// A deck named `name` with one card per entry of `tags` (the card's tag
/// names), in order. Returns the deck's id and its cards' ids.
fn deck_with(store: &mut Store, name: &str, tags: &[&[&str]]) -> (String, Vec<String>) {
    let deck = store.create_deck_at(name, T0).unwrap().id;
    let cards = tags
        .iter()
        .enumerate()
        .map(|(i, names)| {
            let names: Vec<String> = names.iter().map(|&n| n.to_owned()).collect();
            store
                .add_card_with_tags_at(&deck, &format!("{name} {i}"), "a", &names, T0)
                .unwrap()
                .card
                .id
        })
        .collect();
    (deck, cards)
}

fn tag_input(store: &Store, name: &str) -> StudyInput {
    store
        .study_input(&StudyScope::tag(tag_id(store, name)), NOW, DAY_START)
        .unwrap()
}

fn deck_input(store: &Store, deck: &str) -> StudyInput {
    store
        .study_input(&StudyScope::deck(deck), NOW, DAY_START)
        .unwrap()
}

fn review_ids(input: &StudyInput) -> Vec<String> {
    input.reviews.iter().map(|d| d.card_id.clone()).collect()
}

#[test]
fn a_tag_gathers_its_cards_from_every_deck_in_one_order() {
    let mut store = Store::in_memory();
    let (_, a) = deck_with(&mut store, "A", &[&["verbs"], &["nouns"], &["verbs"]]);
    let (_, b) = deck_with(&mut store, "B", &[&["verbs"], &[], &["Verbs", "x"]]);
    learn(&mut store, &a[0], 2);
    learn(&mut store, &b[0], 1);
    learn(&mut store, &a[1], 0);
    let input = tag_input(&store, "verbs");
    // b[0] is the more overdue; a[1] has another tag.
    assert_eq!(review_ids(&input), [b[0].clone(), a[0].clone()]);
    // New cards oldest first, across the decks ("Verbs" is the same tag).
    assert_eq!(input.new, [a[2].clone(), b[2].clone()]);
    assert_eq!(input.steps, []);
}

#[test]
fn each_deck_gives_at_most_what_is_left_of_its_own_limits() {
    let mut store = Store::in_memory();
    store.set_max_new_per_day(2).unwrap();
    store.set_max_reviews_per_day(2).unwrap();
    let (deck_a, a) = deck_with(&mut store, "A", &[&["t"][..]; 6]);
    let (deck_b, b) = deck_with(&mut store, "B", &[&["t"][..]; 3]);
    for (i, card) in a[..4].iter().enumerate() {
        learn(&mut store, card, i64::try_from(i).unwrap());
    }
    // One of A's reviews and one of its new cards already done today.
    answer(&mut store, &a[0], Grade::Pass, NOW - 60_000);
    answer(&mut store, &a[4], Grade::Pass, NOW - 60_000);
    let input = tag_input(&store, "t");
    // A has one review left (its most overdue still due) and one new card;
    // B has no review due and two new cards.
    assert_eq!(review_ids(&input), [a[1].clone()]);
    assert_eq!(input.new, [a[5].clone(), b[0].clone(), b[1].clone()]);
    // a[4] is in its step: never limited.
    assert_eq!(input.steps.len(), 1);
    assert_eq!(input.steps[0].card_id, a[4]);
    assert_eq!((input.new_today, input.reviews_today), (1, 1));
    // Never more than each deck's own session.
    assert_eq!(review_ids(&deck_input(&store, &deck_a)), [a[1].clone()]);
    assert_eq!(deck_input(&store, &deck_a).new, [a[5].clone()]);
    assert_eq!(
        deck_input(&store, &deck_b).new,
        [b[0].clone(), b[1].clone()]
    );
}

#[test]
fn a_card_studied_by_tag_uses_its_decks_limit_for_the_day() {
    let mut store = Store::in_memory();
    store.set_max_new_per_day(2).unwrap();
    // More tagged new cards than the limit: a tag allowance that forgot
    // what the deck did today would offer two of them below.
    let (deck, cards) = deck_with(&mut store, "A", &[&["t"], &[], &["t"], &["t"], &["t"]]);
    assert_eq!(
        deck_input(&store, &deck).new,
        [cards[0].clone(), cards[1].clone()]
    );
    assert_eq!(
        tag_input(&store, "t").new,
        [cards[0].clone(), cards[2].clone()]
    );
    // A tag session introduces cards[0], written to the log as usual.
    answer(&mut store, &cards[0], Grade::Pass, NOW);
    // The deck has one new card left today, and so has the tag.
    let input = deck_input(&store, &deck);
    assert_eq!(input.new, [cards[1].clone()]);
    assert_eq!(input.new_today, 1);
    assert_eq!(tag_input(&store, "t").new, [cards[2].clone()]);
    answer(&mut store, &cards[2], Grade::Pass, NOW);
    // Both used up, by the tag session: the deck shows none.
    assert_eq!(deck_input(&store, &deck).new, Vec::<String>::new());
    assert_eq!(tag_input(&store, "t").new, Vec::<String>::new());
}

#[test]
fn archived_and_deleted_decks_are_left_out_and_come_back_with_them() {
    let mut store = Store::in_memory();
    let (_, live_cards) = deck_with(&mut store, "Live", &[&["t"]]);
    let (archived, archived_cards) = deck_with(&mut store, "Archived", &[&["t"], &["t"]]);
    let (deleted, _) = deck_with(&mut store, "Deleted", &[&["t"]]);
    learn(&mut store, &archived_cards[0], 0);
    answer(&mut store, &archived_cards[1], Grade::Fail, NOW);
    store.archive_deck_at(&archived, NOW).unwrap();
    store.delete_deck_at(&deleted, NOW).unwrap();
    let tag = StudyScope::tag(tag_id(&store, "t"));
    let input = tag_input(&store, "t");
    assert_eq!(input.new, live_cards);
    assert!(input.reviews.is_empty() && input.steps.is_empty());
    // Nor does an archived card's due time wake the session.
    assert_eq!(store.next_study_change(&tag, T0).unwrap(), None);

    store.unarchive_deck(&archived).unwrap();
    let input = tag_input(&store, "t");
    assert_eq!(review_ids(&input), [archived_cards[0].clone()]);
    assert_eq!(input.steps.len(), 1);
    assert!(store.next_study_change(&tag, T0).unwrap().is_some());
}

#[test]
fn suspended_and_deleted_cards_are_left_out() {
    let mut store = Store::in_memory();
    let (_, cards) = deck_with(&mut store, "A", &[&["t"], &["t"], &["t"], &["t"]]);
    learn(&mut store, &cards[0], 0);
    learn(&mut store, &cards[1], 1);
    store.set_suspended(&[cards[0].clone()], true).unwrap();
    store.delete_card_at(&cards[1], NOW).unwrap();
    store.set_suspended(&[cards[2].clone()], true).unwrap();
    let input = tag_input(&store, "t");
    assert!(input.reviews.is_empty() && input.steps.is_empty());
    assert_eq!(input.new, [cards[3].clone()]);
    let tag = StudyScope::tag(tag_id(&store, "t"));
    assert_eq!(store.next_study_change(&tag, NOW).unwrap(), None);
}

/// The next change of a tag skips suspended and deleted cards, asked at a
/// time before every card's due time, so each of them would be the answer
/// if it counted.
#[test]
fn the_next_change_of_a_tag_leaves_out_suspended_and_deleted_cards() {
    let mut store = Store::in_memory();
    let (_, cards) = deck_with(&mut store, "A", &[&["t"], &["t"], &["t"]]);
    // In their step: due 10 minutes after their answer, counted from
    // 20 minutes before that.
    answer(&mut store, &cards[0], Grade::Fail, NOW);
    answer(&mut store, &cards[1], Grade::Fail, NOW + 1_000);
    answer(&mut store, &cards[2], Grade::Fail, NOW + 2_000);
    let tag = StudyScope::tag(tag_id(&store, "t"));
    let before = NOW - DAY_MS;
    let change = |at: i64| at + STEP_MS - EARLY_MS;
    assert_eq!(
        store.next_study_change(&tag, before).unwrap(),
        Some(change(NOW))
    );
    store
        .set_suspended(std::slice::from_ref(&cards[0]), true)
        .unwrap();
    assert_eq!(
        store.next_study_change(&tag, before).unwrap(),
        Some(change(NOW + 1_000))
    );
    store.delete_card_at(&cards[1], NOW).unwrap();
    assert_eq!(
        store.next_study_change(&tag, before).unwrap(),
        Some(change(NOW + 2_000))
    );
}

#[test]
fn a_tag_on_no_live_card_gives_nothing_and_an_unknown_tag_is_reported() {
    let mut store = Store::in_memory();
    let (_, cards) = deck_with(&mut store, "A", &[&["gone"], &[]]);
    store.delete_card_at(&cards[0], NOW).unwrap();
    assert!(tag_input(&store, "gone").is_empty());
    assert_eq!(tag_input(&store, "gone"), StudyInput::default());
    // A tag taken off its last card.
    let other = store.add_tag(&cards[1], "typo").unwrap();
    assert!(store.remove_tag(&cards[1], &other.id).unwrap());
    assert!(tag_input(&store, "typo").is_empty());

    let missing = StudyScope::tag("01920000-0000-7000-8000-00000000dead");
    assert!(matches!(
        store.study_input(&missing, NOW, DAY_START),
        Err(Error::TagNotFound(_))
    ));
    assert!(matches!(
        store.next_study_change(&missing, NOW),
        Err(Error::TagNotFound(_))
    ));
}

#[test]
fn the_next_change_of_a_tag_is_its_earliest_card() {
    let mut store = Store::in_memory();
    let (_, a) = deck_with(&mut store, "A", &[&["t"], &[]]);
    let (_, b) = deck_with(&mut store, "B", &[&["t"]]);
    let tag = StudyScope::tag(tag_id(&store, "t"));
    assert_eq!(store.next_study_change(&tag, NOW).unwrap(), None);
    // Untagged a[1] fails first: its step is not the tag's.
    answer(&mut store, &a[1], Grade::Fail, NOW);
    assert_eq!(store.next_study_change(&tag, NOW).unwrap(), None);
    answer(&mut store, &b[0], Grade::Fail, NOW + 1000);
    answer(&mut store, &a[0], Grade::Fail, NOW + 2000);
    assert_eq!(
        store.next_study_change(&tag, NOW - EARLY_MS).unwrap(),
        Some(NOW + 1000 + STEP_MS - EARLY_MS)
    );
}

#[test]
fn a_tag_on_every_card_of_a_deck_studies_exactly_that_deck() {
    let mut store = Store::in_memory();
    store.set_max_new_per_day(3).unwrap();
    store.set_max_reviews_per_day(4).unwrap();
    let (deck, cards) = deck_with(&mut store, "A", &[&["all"][..]; 12]);
    for (i, card) in cards[..7].iter().enumerate() {
        learn(&mut store, card, i64::try_from(i % 3).unwrap());
    }
    answer(&mut store, &cards[0], Grade::Fail, NOW - 60_000);
    answer(&mut store, &cards[7], Grade::Pass, NOW - 30_000);
    assert_eq!(tag_input(&store, "all"), deck_input(&store, &deck));
    let tag = StudyScope::tag(tag_id(&store, "all"));
    assert_eq!(
        store.next_study_change(&tag, NOW).unwrap(),
        store
            .next_study_change(&StudyScope::deck(&deck), NOW)
            .unwrap()
    );
}

/// A random database for the property: cards in three decks, some tagged,
/// some learned, failed, suspended or deleted, one deck perhaps archived.
#[derive(Debug, Clone)]
struct Spec {
    /// Per card: deck, tagged, history (0 new, 1 learned, 2 learned and
    /// reviewed today, 3 failed today), learned on day, suspended, deleted.
    cards: Vec<(usize, bool, u8, i64, bool, bool)>,
    max_new: u32,
    max_reviews: u32,
    archived: Option<usize>,
}

fn spec() -> impl Strategy<Value = Spec> {
    (
        prop::collection::vec(
            (
                0..3_usize,
                any::<bool>(),
                0..4_u8,
                0..30_i64,
                prop::bool::weighted(0.1),
                prop::bool::weighted(0.1),
            ),
            0..24,
        ),
        0..5_u32,
        0..5_u32,
        prop::option::of(0..3_usize),
    )
        .prop_map(|(cards, max_new, max_reviews, archived)| Spec {
            cards,
            max_new,
            max_reviews,
            archived,
        })
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(48))]

    /// Against the database: a tag session never takes more reviews or new
    /// cards from a deck than that deck's own session, its step cards are
    /// the deck's tagged ones, and its part of each deck is the first of
    /// the deck's due tagged cards within the deck's allowance.
    #[test]
    fn a_tag_session_never_takes_more_from_a_deck_than_the_deck(spec in spec()) {
        let mut store = Store::in_memory();
        store.set_max_new_per_day(spec.max_new).unwrap();
        store.set_max_reviews_per_day(spec.max_reviews).unwrap();
        let decks: Vec<String> = ["A", "B", "C"]
            .iter()
            .map(|n| store.create_deck_at(n, T0).unwrap().id)
            .collect();
        let mut tagged = HashSet::new();
        let mut deck_of = std::collections::HashMap::new();
        for (i, &(deck, has_tag, history, day, suspended, deleted)) in spec.cards.iter().enumerate() {
            let tags = if has_tag { vec!["t".to_owned()] } else { vec!["u".to_owned()] };
            let id = store
                .add_card_with_tags_at(&decks[deck], &format!("q{i}"), "a", &tags, T0)
                .unwrap()
                .card
                .id;
            match history {
                1 => learn(&mut store, &id, day),
                2 => {
                    learn(&mut store, &id, day);
                    answer(&mut store, &id, Grade::Pass, NOW - 60_000);
                }
                3 => {
                    answer(&mut store, &id, Grade::Fail, NOW - 60_000);
                }
                _ => {}
            }
            if suspended {
                store.set_suspended(std::slice::from_ref(&id), true).unwrap();
            }
            if deleted {
                store.delete_card_at(&id, NOW).unwrap();
            }
            if has_tag && !suspended && !deleted {
                tagged.insert(id.clone());
            }
            deck_of.insert(id, deck);
        }
        if let Some(a) = spec.archived {
            store.archive_deck_at(&decks[a], NOW).unwrap();
        }
        let Some(tag) = store.tags().unwrap().into_iter().find(|t| t.name == "t") else {
            // No card was tagged.
            return Ok(());
        };
        let input = store.study_input(&StudyScope::tag(&tag.id), NOW, DAY_START).unwrap();
        let mut seen = HashSet::new();
        for id in input
            .reviews
            .iter()
            .chain(&input.steps)
            .map(|d| &d.card_id)
            .chain(&input.new)
        {
            prop_assert!(tagged.contains(id), "untagged, suspended or deleted {id}");
            prop_assert!(Some(deck_of[id]) != spec.archived, "archived {id}");
            prop_assert!(seen.insert(id.clone()), "twice: {id}");
        }
        for (d, deck) in decks.iter().enumerate() {
            if Some(d) == spec.archived {
                continue;
            }
            let own = deck_input(&store, deck);
            let mine = |ids: Vec<String>| -> Vec<String> {
                ids.into_iter().filter(|id| deck_of[id] == d).collect()
            };
            let tag_reviews = mine(review_ids(&input));
            let tag_new = mine(input.new.clone());
            let tag_steps = mine(input.steps.iter().map(|s| s.card_id.clone()).collect());
            prop_assert!(tag_reviews.len() <= own.reviews.len());
            prop_assert!(tag_new.len() <= own.new.len());
            let own_steps: Vec<String> = own
                .steps
                .iter()
                .map(|s| s.card_id.clone())
                .filter(|id| tagged.contains(id))
                .collect();
            prop_assert_eq!(&tag_steps, &own_steps);
            // The deck's whole due list, without limits, cut to what is
            // left of its own allowance.
            let left_reviews =
                usize::try_from(spec.max_reviews.saturating_sub(own.reviews_today)).unwrap();
            let left_new = usize::try_from(spec.max_new.saturating_sub(own.new_today)).unwrap();
            store.set_max_new_per_day(9_999).unwrap();
            store.set_max_reviews_per_day(9_999).unwrap();
            let all = deck_input(&store, deck);
            store.set_max_new_per_day(spec.max_new).unwrap();
            store.set_max_reviews_per_day(spec.max_reviews).unwrap();
            let want_reviews: Vec<String> = review_ids(&all)
                .into_iter()
                .filter(|id| tagged.contains(id))
                .take(left_reviews)
                .collect();
            let want_new: Vec<String> = all
                .new
                .into_iter()
                .filter(|id| tagged.contains(id))
                .take(left_new)
                .collect();
            prop_assert_eq!(tag_reviews, want_reviews);
            prop_assert_eq!(tag_new, want_new);
        }
    }
}
