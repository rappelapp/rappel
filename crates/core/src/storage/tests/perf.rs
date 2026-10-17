//! Speed at the scale of issue #42: one deck of 5,000 cards, 2,000 of
//! them due, as `fill_scratch_db 5000 --due 2000` makes. The bounds are
//! for a debug test build, about 10 times what was measured (ADR 0025);
//! set `RAPPEL_TIMINGS=1` to print the measurements.

use std::time::{Duration, Instant};

use super::*;
use crate::domain::Grade;
use crate::scheduling::STEP_MS;
use crate::test_support::{DAY_MS, T0};

const CARDS: usize = 5_000;
const DUE: usize = 2_000;

/// "Now" in the tests: the reviews are 40 and 20 days before it.
const NOW: i64 = T0 + 60 * DAY_MS;

/// The fastest of `runs` runs of `f`, and its last result.
fn fastest<T>(what: &str, runs: usize, mut f: impl FnMut() -> T) -> (Duration, T) {
    let mut best = Duration::MAX;
    let mut value = None;
    for _ in 0..runs {
        let start = Instant::now();
        value = Some(f());
        best = best.min(start.elapsed());
    }
    if std::env::var_os("RAPPEL_TIMINGS").is_some() {
        eprintln!("{what}: {best:?}");
    }
    (best, value.unwrap())
}

/// A deck as `fill_scratch_db 5000 --due 2000` makes it: the first 2,000
/// cards learned 40 days before [`NOW`] (every third also lapsed 20 days
/// before), so they are all overdue; the rest new.
fn scratch_store() -> (Store, String) {
    let mut store = Store::in_memory();
    let deck = store.create_deck_at("Scratch", T0).unwrap().id;
    // The default daily limit (200) would hide most of the 2,000 due cards.
    store.set_max_reviews_per_day(9_999).unwrap();
    for i in 0..CARDS {
        let card = store
            .add_card_at(&deck, &format!("Card {i}"), "back", T0)
            .unwrap()
            .id;
        if i < DUE {
            let learned = NOW - 40 * DAY_MS;
            let mut reviews = vec![(Grade::Pass, learned), (Grade::Pass, learned + STEP_MS)];
            if i % 3 == 0 {
                let lapsed = NOW - 20 * DAY_MS;
                reviews.extend([(Grade::Fail, lapsed), (Grade::Pass, lapsed + STEP_MS)]);
            }
            for (grade, at) in reviews {
                store
                    .record_review(&card, grade, Duration::from_secs(8), at)
                    .unwrap();
            }
        }
    }
    (store, deck)
}

#[test]
fn deck_reads_are_fast_at_five_thousand_cards() {
    let (store, deck) = scratch_store();
    let day_start = NOW - DAY_MS / 2;

    let (list, decks) = fastest("list_decks", 3, || {
        store.list_decks(NOW, day_start).unwrap()
    });
    assert_eq!(
        (decks[0].new_count, decks[0].due_count),
        (20, u32::try_from(DUE).unwrap())
    );
    let (input, study) = fastest("study_input", 3, || {
        store.study_input(&deck, NOW, day_start).unwrap()
    });
    assert_eq!(study.reviews.len(), DUE);
    let (next, _) = fastest("next_study_change", 3, || {
        store.next_study_change(&deck, NOW).unwrap()
    });
    let (cards, listed) = fastest("list_cards", 3, || store.list_cards(&deck).unwrap());
    assert_eq!(listed.len(), CARDS);
    let (all, _) = fastest("decks", 3, || store.decks().unwrap());
    let (one, _) = fastest("deck", 3, || store.deck(&deck).unwrap());

    // What the deck page does on every change (#42 item 2).
    assert!(
        input + next < Duration::from_millis(STUDY_MS),
        "{input:?} + {next:?}"
    );
    assert!(list < Duration::from_millis(STUDY_MS), "{list:?}");
    assert!(cards < Duration::from_millis(LIST_CARDS_MS), "{cards:?}");
    // The point of `deck` / `decks` (#42 item 1): finding a deck or the
    // names does not count every card, so it is far cheaper than the
    // counted list whatever the machine's speed.
    assert!(all * 10 < list, "decks {all:?}, list_decks {list:?}");
    assert!(one * 10 < list, "deck {one:?}, list_decks {list:?}");
}

#[test]
fn listing_five_thousand_tagged_cards_is_fast() {
    let (mut store, deck) = scratch_store();
    let ids: Vec<String> = store
        .list_cards(&deck)
        .unwrap()
        .into_iter()
        .map(|c| c.id)
        .collect();
    for (i, id) in ids.iter().enumerate() {
        for name in [format!("tag {}", i % 40), format!("group {}", i % 7)] {
            store.add_tag_at(id, &name, T0).unwrap();
        }
    }
    let (tagged, listed) = fastest("list_tagged_cards", 3, || {
        store.list_tagged_cards(&deck).unwrap()
    });
    assert_eq!(listed.len(), CARDS);
    assert!(listed.iter().all(|c| c.tags.len() == 2));
    assert!(tagged < Duration::from_millis(LIST_TAGGED_MS), "{tagged:?}");
}

#[test]
fn planning_a_five_thousand_card_import_is_fast() {
    let (store, _) = scratch_store();
    let bytes = store.export_bytes_at(NOW).unwrap().0;
    let target = Store::in_memory();
    let (plan, summary) = fastest("plan_import (5,000 cards)", 3, || {
        target
            .plan_import_bytes(&bytes, &crate::interchange::Limits::DEFAULT)
            .unwrap()
            .summary()
            .clone()
    });
    assert_eq!(summary.cards.new, CARDS);
    assert!(plan < Duration::from_millis(PLAN_IMPORT_MS), "{plan:?}");
}

/// `study_input` + `next_study_change` (measured 14 ms), and `list_decks`
/// (12 ms), each.
const STUDY_MS: u64 = 150;
/// `list_cards` of 5,000 cards (measured 6 ms).
const LIST_CARDS_MS: u64 = 100;
/// `list_tagged_cards` of 5,000 cards with two tags each (measured 30 ms).
const LIST_TAGGED_MS: u64 = 300;
/// Planning the import of the 5,000-card export into an empty store
/// (measured 80 ms).
const PLAN_IMPORT_MS: u64 = 1_000;
