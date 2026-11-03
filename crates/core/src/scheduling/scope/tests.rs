use std::collections::{HashMap, HashSet};

use proptest::prelude::*;

use super::*;
use crate::scheduling::session::Session;
use crate::test_support::{DAY_MS, T0};

const NOW: i64 = T0 + 30 * DAY_MS;

fn allowance(deck: &str, new: u32, reviews: u32) -> DeckAllowance {
    DeckAllowance {
        deck_id: deck.to_owned(),
        new,
        reviews,
        new_today: 0,
        reviews_today: 0,
    }
}

fn card(id: &str, deck: &str, seq: i64, stage: Stage) -> Candidate {
    Candidate {
        card_id: id.to_owned(),
        deck_id: deck.to_owned(),
        seq,
        suspended: false,
        deleted: false,
        stage,
    }
}

fn review_ids(input: &StudyInput) -> Vec<&str> {
    input.reviews.iter().map(|d| d.card_id.as_str()).collect()
}

fn step_ids(input: &StudyInput) -> Vec<&str> {
    input.steps.iter().map(|d| d.card_id.as_str()).collect()
}

fn new_ids(input: &StudyInput) -> Vec<&str> {
    input.new.iter().map(String::as_str).collect()
}

#[test]
fn the_tag_on_no_card_gives_an_empty_session() {
    let input = allocate(&[allowance("a", 20, 200)], Vec::new(), NOW);
    assert!(input.is_empty());
    assert_eq!(input, StudyInput::default());
    assert!(allocate(&[], Vec::new(), NOW).is_empty());
}

#[test]
fn each_deck_gives_at_most_its_own_allowance() {
    let cards = vec![
        card("a1", "a", 1, Stage::Review(NOW - 3 * DAY_MS)),
        card("a2", "a", 2, Stage::Review(NOW - 2 * DAY_MS)),
        card("a3", "a", 3, Stage::Review(NOW - DAY_MS)),
        card("an1", "a", 4, Stage::New),
        card("an2", "a", 5, Stage::New),
        card("b1", "b", 6, Stage::Review(NOW - 10 * DAY_MS)),
        card("b2", "b", 7, Stage::Review(NOW - 9 * DAY_MS)),
        card("bn1", "b", 8, Stage::New),
        card("bn2", "b", 9, Stage::New),
    ];
    let input = allocate(&[allowance("a", 1, 2), allowance("b", 2, 1)], cards, NOW);
    // a keeps its two most overdue, b its most overdue; merged most
    // overdue first.
    assert_eq!(review_ids(&input), ["b1", "a1", "a2"]);
    // a introduces one new card, b two; merged oldest first.
    assert_eq!(new_ids(&input), ["an1", "bn1", "bn2"]);
}

#[test]
fn a_deck_whose_limits_are_reached_gives_only_its_step_cards() {
    let cards = vec![
        card("a1", "a", 1, Stage::Review(NOW - DAY_MS)),
        card("an", "a", 2, Stage::New),
        card("as", "a", 3, Stage::Step(NOW - 1)),
        card("b1", "b", 4, Stage::Review(NOW - DAY_MS)),
        card("bn", "b", 5, Stage::New),
    ];
    let input = allocate(&[allowance("a", 0, 0), allowance("b", 5, 5)], cards, NOW);
    assert_eq!(review_ids(&input), ["b1"]);
    assert_eq!(new_ids(&input), ["bn"]);
    // A failed card always comes back: steps are never limited.
    assert_eq!(step_ids(&input), ["as"]);
}

#[test]
fn only_due_cards_are_taken_and_steps_up_to_twenty_minutes_early() {
    let cards = vec![
        card("due", "a", 1, Stage::Review(NOW)),
        card("later", "a", 2, Stage::Review(NOW + 1)),
        card("step-now", "a", 3, Stage::Step(NOW)),
        card("step-early", "a", 4, Stage::Step(NOW + EARLY_MS)),
        card("step-later", "a", 5, Stage::Step(NOW + EARLY_MS + 1)),
    ];
    let input = allocate(&[allowance("a", 20, 200)], cards, NOW);
    assert_eq!(review_ids(&input), ["due"]);
    assert_eq!(step_ids(&input), ["step-now", "step-early"]);
}

#[test]
fn a_not_yet_due_review_does_not_use_the_allowance() {
    let cards = vec![
        card("later", "a", 1, Stage::Review(NOW + DAY_MS)),
        card("due", "a", 2, Stage::Review(NOW - DAY_MS)),
    ];
    let input = allocate(&[allowance("a", 0, 1)], cards, NOW);
    assert_eq!(review_ids(&input), ["due"]);
}

#[test]
fn suspended_and_deleted_cards_are_never_taken_nor_counted() {
    let mut suspended = card("s", "a", 1, Stage::Review(NOW - 5 * DAY_MS));
    suspended.suspended = true;
    let mut deleted = card("d", "a", 2, Stage::Review(NOW - 4 * DAY_MS));
    deleted.deleted = true;
    let mut new_suspended = card("ns", "a", 3, Stage::New);
    new_suspended.suspended = true;
    let mut step_deleted = card("sd", "a", 4, Stage::Step(NOW));
    step_deleted.deleted = true;
    let cards = vec![
        suspended,
        deleted,
        new_suspended,
        step_deleted,
        card("r", "a", 5, Stage::Review(NOW - DAY_MS)),
        card("n", "a", 6, Stage::New),
    ];
    // An allowance of one each: the left-out cards do not use it.
    let input = allocate(&[allowance("a", 1, 1)], cards, NOW);
    assert_eq!(review_ids(&input), ["r"]);
    assert_eq!(new_ids(&input), ["n"]);
    assert_eq!(input.steps, []);
}

#[test]
fn a_card_whose_deck_is_archived_or_not_in_scope_is_never_taken() {
    // The caller gives no allowance to an archived (or deleted) deck.
    let cards = vec![
        card("arch-r", "archived", 1, Stage::Review(NOW - DAY_MS)),
        card("arch-n", "archived", 2, Stage::New),
        card("arch-s", "archived", 3, Stage::Step(NOW)),
        card("live", "a", 4, Stage::New),
    ];
    let input = allocate(&[allowance("a", 20, 200)], cards, NOW);
    assert_eq!(new_ids(&input), ["live"]);
    assert!(input.reviews.is_empty() && input.steps.is_empty());
}

#[test]
fn ties_go_to_the_older_card_whatever_its_deck() {
    let at = NOW - DAY_MS;
    let cards = vec![
        card("b-old", "b", 1, Stage::Review(at)),
        card("a-new", "a", 4, Stage::Review(at)),
        card("a-old", "a", 2, Stage::Review(at)),
        card("b-mid", "b", 3, Stage::Review(at)),
        card("sb", "b", 6, Stage::Step(NOW)),
        card("sa", "a", 5, Stage::Step(NOW)),
    ];
    let input = allocate(&[allowance("a", 0, 1), allowance("b", 0, 9)], cards, NOW);
    // a's allowance of one keeps its oldest of the tie.
    assert_eq!(review_ids(&input), ["b-old", "a-old", "b-mid"]);
    assert_eq!(step_ids(&input), ["sa", "sb"]);
}

#[test]
fn many_decks_merge_in_one_order() {
    let decks: Vec<String> = (0..50).map(|d| format!("deck{d}")).collect();
    let allowances: Vec<_> = decks.iter().map(|d| allowance(d, 1, 2)).collect();
    let mut cards = Vec::new();
    let mut seq = 0;
    for (d, deck) in decks.iter().enumerate() {
        let d = i64::try_from(d).unwrap();
        for i in 0..4 {
            seq += 1;
            cards.push(card(
                &format!("{deck}-r{i}"),
                deck,
                seq,
                Stage::Review(NOW - (d + i) * 1000),
            ));
            seq += 1;
            cards.push(card(&format!("{deck}-n{i}"), deck, seq, Stage::New));
        }
    }
    let input = allocate(&allowances, cards, NOW);
    assert_eq!(input.reviews.len(), 100);
    assert_eq!(input.new.len(), 50);
    assert!(input.reviews.is_sorted_by_key(|d| d.due_at));
    for deck in &decks {
        let from = |ids: Vec<&str>| {
            ids.iter()
                .filter(|id| id.starts_with(&format!("{deck}-")))
                .count()
        };
        assert_eq!(from(review_ids(&input)), 2, "{deck}");
        assert_eq!(from(new_ids(&input)), 1, "{deck}");
    }
}

#[test]
fn today_counts_add_up_over_the_decks_that_give_a_card() {
    let mut a = allowance("a", 1, 1);
    (a.new_today, a.reviews_today) = (3, 4);
    let mut b = allowance("b", 1, 1);
    (b.new_today, b.reviews_today) = (5, 6);
    let mut idle = allowance("idle", 1, 1);
    (idle.new_today, idle.reviews_today) = (100, 100);
    let cards = vec![
        card("a", "a", 1, Stage::New),
        card("b", "b", 2, Stage::Step(NOW)),
        card("idle", "idle", 3, Stage::Review(NOW + DAY_MS)),
    ];
    let input = allocate(&[a, b, idle], cards, NOW);
    assert_eq!((input.new_today, input.reviews_today), (8, 10));
}

#[test]
fn a_deck_listed_twice_gives_its_cards_once() {
    let cards = vec![card("n", "a", 1, Stage::New)];
    let input = allocate(&[allowance("a", 5, 5), allowance("a", 5, 5)], cards, NOW);
    assert_eq!(new_ids(&input), ["n"]);
}

#[test]
fn a_session_over_a_tag_interleaves_new_cards_like_a_deck_session() {
    let cards = vec![
        card("r1", "a", 1, Stage::Review(NOW - 4 * DAY_MS)),
        card("r2", "b", 2, Stage::Review(NOW - 3 * DAY_MS)),
        card("r3", "a", 3, Stage::Review(NOW - 2 * DAY_MS)),
        card("r4", "b", 4, Stage::Review(NOW - DAY_MS)),
        card("n1", "b", 5, Stage::New),
        card("n2", "a", 6, Stage::New),
    ];
    let input = allocate(&[allowance("a", 9, 9), allowance("b", 9, 9)], cards, NOW);
    let mut session = Session::new(input);
    let mut shown = Vec::new();
    while let Some(id) = session.next(NOW).map(str::to_owned) {
        session.skip(&id);
        shown.push(id);
    }
    assert_eq!(shown, ["r1", "n1", "r2", "r3", "n2", "r4"]);
}

/// A card for the property tests: its deck (of three), stage, flags, and
/// whether it carries the tag.
fn arb_card() -> impl Strategy<Value = (usize, u8, i64, bool, bool, bool)> {
    (
        0..3_usize,
        0..3_u8,
        -3 * DAY_MS..3 * DAY_MS,
        prop::bool::weighted(0.1),
        prop::bool::weighted(0.1),
        any::<bool>(),
    )
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(512))]

    /// A tag session never takes more from a deck than that deck's own
    /// session would (its cards are a subset, cut by the same allowance),
    /// and it is the union of what each deck gives for the tag alone, in
    /// one order.
    #[test]
    fn a_tag_session_is_the_union_of_its_decks_within_their_limits(
        specs in prop::collection::vec(arb_card(), 0..40),
        limits in prop::collection::vec((0..6_u32, 0..6_u32), 3),
        archived in 0..4_usize,
    ) {
        let decks = ["a", "b", "c"];
        let allowances: Vec<DeckAllowance> = decks
            .iter()
            .zip(&limits)
            .enumerate()
            .filter(|&(i, _)| i != archived)
            .map(|(_, (d, &(new, reviews)))| allowance(d, new, reviews))
            .collect();
        let mut all = Vec::new();
        let mut tagged = Vec::new();
        for (seq, &(deck, stage, offset, suspended, deleted, has_tag)) in specs.iter().enumerate() {
            let stage = match stage {
                0 => Stage::New,
                1 => Stage::Review(NOW + offset),
                _ => Stage::Step(NOW + offset / 100),
            };
            let mut c = card(&format!("c{seq}"), decks[deck], i64::try_from(seq).unwrap(), stage);
            c.suspended = suspended;
            c.deleted = deleted;
            if has_tag {
                tagged.push(c.clone());
            }
            all.push(c);
        }
        let deck_of: HashMap<String, String> =
            all.iter().map(|c| (c.card_id.clone(), c.deck_id.clone())).collect();
        let tag = allocate(&allowances, tagged.clone(), NOW);
        let ids = |input: &StudyInput| -> (Vec<String>, Vec<String>, Vec<String>) {
            (
                input.reviews.iter().map(|d| d.card_id.clone()).collect(),
                input.steps.iter().map(|d| d.card_id.clone()).collect(),
                input.new.clone(),
            )
        };
        let (tag_reviews, tag_steps, tag_new) = ids(&tag);
        let from = |list: &[String], deck: &str| -> Vec<String> {
            list.iter().filter(|id| deck_of[*id] == deck).cloned().collect()
        };
        for (i, deck) in decks.iter().enumerate() {
            if i == archived {
                // Nothing of an archived deck.
                prop_assert!(from(&tag_reviews, deck).is_empty());
                prop_assert!(from(&tag_steps, deck).is_empty());
                prop_assert!(from(&tag_new, deck).is_empty());
                continue;
            }
            let own: Vec<_> = allowances.iter().filter(|a| a.deck_id == *deck).cloned().collect();
            let deck_cards: Vec<_> = all.iter().filter(|c| c.deck_id == *deck).cloned().collect();
            let (d_reviews, d_steps, d_new) = ids(&allocate(&own, deck_cards, NOW));
            // Never more than the deck's own session.
            prop_assert!(from(&tag_reviews, deck).len() <= d_reviews.len());
            prop_assert!(from(&tag_new, deck).len() <= d_new.len());
            // Step cards are not limited: exactly the deck's tagged ones.
            let tagged_ids: HashSet<&str> = tagged.iter().map(|c| c.card_id.as_str()).collect();
            let d_tagged_steps: Vec<String> =
                d_steps.into_iter().filter(|id| tagged_ids.contains(id.as_str())).collect();
            prop_assert_eq!(from(&tag_steps, deck), d_tagged_steps);
            // The union: the deck's part equals its session over the tag alone.
            let deck_tagged: Vec<_> = tagged.iter().filter(|c| c.deck_id == *deck).cloned().collect();
            let (p_reviews, p_steps, p_new) = ids(&allocate(&own, deck_tagged, NOW));
            prop_assert_eq!(from(&tag_reviews, deck), p_reviews);
            prop_assert_eq!(from(&tag_steps, deck), p_steps);
            prop_assert_eq!(from(&tag_new, deck), p_new);
        }
        // Every card taken carries the tag, once, live and not suspended.
        let mut seen = HashSet::new();
        for id in tag_reviews.iter().chain(&tag_steps).chain(&tag_new) {
            let c = tagged.iter().find(|c| &c.card_id == id);
            prop_assert!(c.is_some_and(|c| !c.suspended && !c.deleted), "{id}");
            prop_assert!(seen.insert(id.clone()), "{id} twice");
        }
        // One order: most overdue first, earliest step first, oldest new first.
        let seq = |id: &String| tagged.iter().find(|c| &c.card_id == id).map(|c| c.seq);
        prop_assert!(tag.reviews.is_sorted_by_key(|d| (d.due_at, seq(&d.card_id))));
        prop_assert!(tag.steps.is_sorted_by_key(|d| (d.due_at, seq(&d.card_id))));
        prop_assert!(tag.new.is_sorted_by_key(seq));
    }

    /// With the tag on every card of a single deck, the tag session is
    /// that deck's session.
    #[test]
    fn a_tag_on_every_card_of_one_deck_is_that_deck(
        specs in prop::collection::vec(arb_card(), 0..30),
        new in 0..6_u32,
        reviews in 0..6_u32,
    ) {
        let cards: Vec<_> = specs
            .iter()
            .enumerate()
            .map(|(seq, &(_, stage, offset, suspended, deleted, _))| {
                let stage = match stage {
                    0 => Stage::New,
                    1 => Stage::Review(NOW + offset),
                    _ => Stage::Step(NOW + offset / 100),
                };
                let mut c = card(&format!("c{seq}"), "a", i64::try_from(seq).unwrap(), stage);
                c.suspended = suspended;
                c.deleted = deleted;
                c
            })
            .collect();
        let input = allocate(&[allowance("a", new, reviews)], cards.clone(), NOW);
        // The reference: the deck session's rules spelled out.
        let live: Vec<_> = cards.iter().filter(|c| !c.suspended && !c.deleted).collect();
        let mut want_reviews: Vec<_> = live
            .iter()
            .filter_map(|c| match c.stage {
                Stage::Review(at) if at <= NOW => Some((at, c.seq, c.card_id.clone())),
                _ => None,
            })
            .collect();
        want_reviews.sort();
        want_reviews.truncate(reviews as usize);
        let mut want_new: Vec<_> = live
            .iter()
            .filter(|c| c.stage == Stage::New)
            .map(|c| c.card_id.clone())
            .collect();
        want_new.truncate(new as usize);
        prop_assert_eq!(
            input.reviews.iter().map(|d| d.card_id.clone()).collect::<Vec<_>>(),
            want_reviews.into_iter().map(|(_, _, id)| id).collect::<Vec<_>>()
        );
        prop_assert_eq!(input.new, want_new);
    }
}
