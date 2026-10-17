use proptest::prelude::*;

use super::*;
use crate::Error;
use crate::domain::CardState;
use crate::id::IdGen;
use crate::scheduling::{STEP_MS, Scheduler};
use crate::test_support::{DAY_MS, T0, schedule_table, scheduled};

fn store_with_cards(n: usize) -> (Store, String, Vec<String>) {
    let mut store = Store::in_memory();
    let deck = store.create_deck_at("Maths", T0).unwrap().id;
    let cards = (0..n)
        .map(|i| {
            store
                .add_card_at(&deck, &format!("q{i}"), "a", T0)
                .unwrap()
                .id
        })
        .collect();
    (store, deck, cards)
}

fn rebuilt(store: &Store, card: &str) -> CardState {
    Scheduler::default()
        .rebuild_state(card, &store.review_events(card).unwrap())
        .unwrap()
}

#[test]
fn record_review_writes_the_event_and_the_schedule() {
    let (mut store, _, cards) = store_with_cards(1);
    let e = store
        .record_review(&cards[0], Grade::Pass, Duration::from_millis(1234), T0)
        .unwrap();
    assert_eq!(
        (e.card_id.as_str(), e.reviewed_at, e.grade, e.duration_ms),
        (cards[0].as_str(), T0, Grade::Pass, 1234)
    );
    assert_eq!(store.review_events(&cards[0]).unwrap(), [e]);
    let s = scheduled(store.card_state(&cards[0]).unwrap());
    assert_eq!((s.last_review_at, s.due_at, s.reps), (T0, T0 + STEP_MS, 1));
    assert_eq!(
        store.card_state(&cards[0]).unwrap(),
        rebuilt(&store, &cards[0])
    );
}

#[test]
fn a_clock_that_went_backwards_is_floored_to_the_last_review() {
    let (mut store, _, cards) = store_with_cards(1);
    let first = store
        .record_review(&cards[0], Grade::Fail, Duration::ZERO, T0)
        .unwrap();
    let second = store
        .record_review(&cards[0], Grade::Pass, Duration::ZERO, T0 - DAY_MS)
        .unwrap();
    assert_eq!(second.reviewed_at, T0);
    assert!(second.id > first.id);
    assert_eq!(store.review_events(&cards[0]).unwrap(), [first, second]);
    assert_eq!(
        store.card_state(&cards[0]).unwrap(),
        rebuilt(&store, &cards[0])
    );
}

#[test]
fn reviews_in_one_millisecond_keep_their_order() {
    let (mut store, _, cards) = store_with_cards(1);
    let recorded: Vec<_> = [Grade::Fail, Grade::Pass, Grade::Fail, Grade::Pass]
        .into_iter()
        .map(|g| {
            store
                .record_review(&cards[0], g, Duration::ZERO, T0)
                .unwrap()
        })
        .collect();
    assert_eq!(store.review_events(&cards[0]).unwrap(), recorded);
    assert_eq!(
        store.card_state(&cards[0]).unwrap(),
        rebuilt(&store, &cards[0])
    );
}

/// A restart (fresh id generator) with a clock behind the last review
/// still records an event that sorts after it.
#[test]
fn a_restart_with_the_clock_behind_still_appends_last() {
    let (mut store, _, cards) = store_with_cards(1);
    let first = store
        .record_review(&cards[0], Grade::Fail, Duration::ZERO, T0 + DAY_MS)
        .unwrap();
    store.ids = IdGen::new();
    let second = store
        .record_review(&cards[0], Grade::Pass, Duration::ZERO, T0)
        .unwrap();
    assert_eq!(second.reviewed_at, first.reviewed_at);
    assert!(second.id > first.id, "{} <= {}", second.id, first.id);
    assert_eq!(store.review_events(&cards[0]).unwrap(), [first, second]);
}

/// Regression: a previous run (clock far ahead) issued `x` for card A and
/// `x + 1` for card B. After a restart, reviewing A must not issue `x + 1`
/// again, so the generator observes the largest id of the whole log, not
/// just A's last one.
#[test]
fn a_restart_does_not_reissue_the_next_id_of_another_card() {
    let (mut store, _, cards) = store_with_cards(2);
    let x = "7fffffff-ffff-7fff-bfff-fffffffffff0";
    let x_plus_1 = "7fffffff-ffff-7fff-bfff-fffffffffff1";
    store
        .conn
        .execute(
            "INSERT INTO review_events (id, card_id, reviewed_at, grade, duration_ms)
             VALUES (?1, ?2, ?5, 'pass', 0), (?3, ?4, ?5, 'pass', 0)",
            params![x, cards[0], x_plus_1, cards[1], T0],
        )
        .unwrap();
    store.rebuild_schedule_cache(&Scheduler::default()).unwrap();
    store.ids = IdGen::new();

    let e = store
        .record_review(&cards[0], Grade::Pass, Duration::ZERO, T0)
        .unwrap();
    assert_eq!(e.id, "7fffffff-ffff-7fff-bfff-fffffffffff2");
    assert_eq!(
        store.card_state(&cards[0]).unwrap(),
        rebuilt(&store, &cards[0])
    );
}

/// A stored id that is not a UUID (the schema only checks length and
/// case) as the largest id refuses new ids in that table, and the refused
/// write leaves nothing behind.
#[test]
fn a_malformed_largest_id_refuses_the_write_and_rolls_back() {
    let (mut store, deck, cards) = store_with_cards(1);
    let bad = "zzzzzzzz-zzzz-zzzz-zzzz-zzzzzzzzzzzz";

    store
        .conn
        .execute(
            "INSERT INTO review_events (id, card_id, reviewed_at, grade, duration_ms)
             VALUES (?1, ?2, ?3, 'pass', 0)",
            params![bad, cards[0], T0],
        )
        .unwrap();
    store.rebuild_schedule_cache(&Scheduler::default()).unwrap();
    let before = schedule_table(&store);
    let err = store
        .record_review(&cards[0], Grade::Pass, Duration::ZERO, T0)
        .unwrap_err();
    assert!(
        matches!(&err, Error::InvalidColumn { column: "review_events.id", value } if value == bad),
        "{err}"
    );
    assert_eq!(store.review_events(&cards[0]).unwrap().len(), 1);
    assert_eq!(schedule_table(&store), before);

    store
        .conn
        .execute(
            "INSERT INTO cards (id, deck_id, front_md, back_md, created_at, updated_at)
             VALUES (?1, ?2, 'q', 'a', 1, 1)",
            params![bad, deck],
        )
        .unwrap();
    let err = store.add_card_at(&deck, "new", "a", T0).unwrap_err();
    assert!(
        matches!(
            err,
            Error::InvalidColumn {
                column: "cards.id",
                ..
            }
        ),
        "{err}"
    );
    assert_eq!(store.list_cards(&deck).unwrap().len(), 2);

    store
        .conn
        .execute(
            "INSERT INTO decks (id, name, name_key, created_at, updated_at)
             VALUES (?1, 'Bad', 'bad', 1, 1)",
            [bad],
        )
        .unwrap();
    let err = store.create_deck_at("Physics", T0).unwrap_err();
    assert!(
        matches!(
            err,
            Error::InvalidColumn {
                column: "decks.id",
                ..
            }
        ),
        "{err}"
    );
    assert_eq!(store.list_decks(T0, T0).unwrap().len(), 2);
}

#[test]
fn duration_is_stored_in_milliseconds_saturating() {
    let (mut store, _, cards) = store_with_cards(1);
    store
        .record_review(&cards[0], Grade::Pass, Duration::MAX, T0)
        .unwrap();
    assert_eq!(
        store.review_events(&cards[0]).unwrap()[0].duration_ms,
        i64::MAX
    );
}

#[test]
fn missing_cards_cannot_be_reviewed() {
    let (mut store, _, _) = store_with_cards(0);
    let missing = "01920000-0000-7000-8000-00000000dead";
    assert!(matches!(
        store.record_review(missing, Grade::Pass, Duration::ZERO, T0),
        Err(Error::CardNotFound(_))
    ));
}

/// The scheduler refusing the review (here: a cache dated after the log)
/// writes nothing.
#[test]
fn a_refused_review_writes_nothing() {
    let (mut store, _, cards) = store_with_cards(1);
    store
        .conn
        .execute(
            "INSERT INTO card_schedule VALUES (?1, 'review', ?2, 3.0, 5.0, ?2, 1, 0)",
            params![cards[0], T0 + DAY_MS],
        )
        .unwrap();
    let before = schedule_table(&store);
    let err = store
        .record_review(&cards[0], Grade::Pass, Duration::ZERO, T0)
        .unwrap_err();
    assert!(matches!(err, Error::ReviewBeforeLastReview { .. }), "{err}");
    assert_eq!(store.review_events(&cards[0]).unwrap().len(), 0);
    assert_eq!(schedule_table(&store), before);
}

/// A failure after the event was inserted (the cache write) rolls the
/// event back too: both happen in one transaction.
#[test]
fn a_failed_cache_write_rolls_back_the_event() {
    let (mut store, _, cards) = store_with_cards(1);
    store
        .conn
        .execute_batch(
            "CREATE TEMP TRIGGER fail_cache BEFORE INSERT ON card_schedule
             BEGIN SELECT RAISE(ABORT, 'cache write failed'); END;",
        )
        .unwrap();
    let err = store
        .record_review(&cards[0], Grade::Pass, Duration::ZERO, T0)
        .unwrap_err();
    assert!(matches!(err, Error::Sqlite(_)), "{err}");
    assert_eq!(store.review_events(&cards[0]).unwrap().len(), 0);
    assert_eq!(schedule_table(&store).len(), 0);
}

#[derive(Debug, Clone)]
enum Step {
    Review {
        /// Taken modulo the number of cards made so far.
        card: usize,
        pass: bool,
        clock_delta: i64,
        duration_ms: u64,
    },
    /// A new process: the id generator starts from scratch.
    Restart,
    AddCard,
    /// Soft-deletes a card (modulo the number of cards).
    DeleteCard(usize),
    /// Soft-deletes the deck with its live cards.
    DeleteDeck,
    /// Brings the deck back with exactly the cards deleted with it.
    UndeleteDeck,
}

fn step() -> impl Strategy<Value = Step> {
    let delta = prop_oneof![
        3 => Just(0i64),
        2 => -2 * DAY_MS..0,
        3 => 0..=3 * STEP_MS,
        3 => 0..=60 * DAY_MS,
    ];
    let review = (0usize..64, any::<bool>(), delta, 0u64..120_000).prop_map(
        |(card, pass, clock_delta, duration_ms)| Step::Review {
            card,
            pass,
            clock_delta,
            duration_ms,
        },
    );
    prop_oneof![
        30 => review,
        3 => Just(Step::Restart),
        3 => Just(Step::AddCard),
        2 => (0usize..64).prop_map(Step::DeleteCard),
        1 => Just(Step::DeleteDeck),
        1 => Just(Step::UndeleteDeck),
    ]
}

/// What the store should hold: each card's recorded events and whether it
/// was deleted on its own, and which cards went with a deck delete.
struct Model {
    events: Vec<Vec<ReviewEvent>>,
    deleted_alone: Vec<bool>,
    deck_deleted: bool,
}

impl Model {
    fn live(&self, card: usize) -> bool {
        !self.deck_deleted && !self.deleted_alone[card]
    }
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(256))]

    /// The cache written review by review equals a full rebuild from the
    /// log, whatever the clock does, however often the app restarts, and
    /// whatever cards and decks are added, deleted and brought back.
    /// Deleted cards refuse reviews and keep their history.
    #[test]
    fn cache_equals_rebuild_after_random_reviews(
        n_cards in 1usize..=4,
        steps in prop::collection::vec(step(), 1..200),
    ) {
        let (mut store, deck, mut cards) = store_with_cards(n_cards);
        let mut clock = T0;
        let mut model = Model {
            events: vec![Vec::new(); n_cards],
            deleted_alone: vec![false; n_cards],
            deck_deleted: false,
        };
        for step in steps {
            match step {
                Step::Restart => store.ids = IdGen::new(),
                Step::Review { card, pass, clock_delta, duration_ms } => {
                    clock += clock_delta;
                    let card = card % cards.len();
                    let grade = if pass { Grade::Pass } else { Grade::Fail };
                    let result = store.record_review(
                        &cards[card], grade, Duration::from_millis(duration_ms), clock,
                    );
                    if model.live(card) {
                        let e = result.unwrap();
                        let floor = model.events[card]
                            .last()
                            .map_or(clock, |l| l.reviewed_at.max(clock));
                        prop_assert_eq!(e.reviewed_at, floor);
                        model.events[card].push(e);
                    } else {
                        prop_assert!(matches!(result, Err(Error::CardNotFound(_))));
                    }
                }
                Step::AddCard => {
                    let result = store.add_card_at(&deck, "q", "a", clock);
                    if model.deck_deleted {
                        prop_assert!(matches!(result, Err(Error::DeckNotFound(_))));
                    } else {
                        cards.push(result.unwrap().id);
                        model.events.push(Vec::new());
                        model.deleted_alone.push(false);
                    }
                }
                Step::DeleteCard(card) => {
                    let card = card % cards.len();
                    let result = store.delete_card_at(&cards[card], clock);
                    if model.live(card) {
                        result.unwrap();
                        model.deleted_alone[card] = true;
                    } else {
                        prop_assert!(matches!(result, Err(Error::CardNotFound(_))));
                    }
                }
                Step::DeleteDeck => {
                    let result = store.delete_deck_at(&deck, clock);
                    if model.deck_deleted {
                        prop_assert!(matches!(result, Err(Error::DeckNotFound(_))));
                    } else {
                        result.unwrap();
                        model.deck_deleted = true;
                    }
                }
                Step::UndeleteDeck => {
                    store.undelete_deck(&deck).unwrap();
                    model.deck_deleted = false;
                }
            }
        }

        for (i, (card, events)) in cards.iter().zip(&model.events).enumerate() {
            // Liveness as the model says: a live card reads back.
            prop_assert_eq!(store.card(card).is_ok(), model.live(i));
            // (c) The log replays in the order it was recorded, deleted
            // cards included.
            prop_assert_eq!(&store.review_events(card).unwrap(), events);
            // (a) The cache equals the rebuild of this card's log.
            prop_assert_eq!(store.card_state(card).unwrap(), rebuilt(&store, card));
        }
        // (b) Rebuilding the whole cache changes nothing.
        let before = schedule_table(&store);
        store.rebuild_schedule_cache(&Scheduler::default()).unwrap();
        prop_assert_eq!(schedule_table(&store), before);
    }
}

mod study {
    use std::time::Duration;

    use super::{DAY_MS, T0, rebuilt, store_with_cards};
    use crate::domain::{Grade, Phase};
    use crate::scheduling::STEP_MS;
    use crate::scheduling::session::{Due, EARLY_MS, Session};

    /// The default `max_new_per_day`.
    const NEW_PER_DAY: u32 = 20;
    use crate::test_support::answer;

    /// "Today" began two hours before `T0`.
    const DAY_START: i64 = T0 - 2 * 3_600_000;

    #[test]
    fn new_cards_are_limited_per_day_from_first_reviews() {
        let (mut store, deck, cards) = store_with_cards(30);
        let input = store.study_input(&deck, T0, DAY_START).unwrap();
        assert_eq!(input.new_today, 0);
        assert_eq!(input.new, cards[..NEW_PER_DAY as usize]);

        // First reviewed yesterday (just before 4 a.m.): not today's.
        answer(&mut store, &cards[0], Grade::Pass, DAY_START - 1);
        // First reviewed at the very start of today, then again later: once.
        answer(&mut store, &cards[1], Grade::Pass, DAY_START);
        answer(&mut store, &cards[1], Grade::Pass, T0);
        // First reviewed today, then deleted: still used today's budget.
        answer(&mut store, &cards[2], Grade::Fail, T0);
        store.delete_card_at(&cards[2], T0).unwrap();
        // Yesterday's card reviewed again today does not count again.
        answer(&mut store, &cards[0], Grade::Pass, T0);

        let input = store.study_input(&deck, T0, DAY_START).unwrap();
        assert_eq!(input.new_today, 2);
        assert_eq!(input.new, cards[3..3 + NEW_PER_DAY as usize - 2]);
    }

    #[test]
    fn a_full_day_offers_no_new_cards_and_other_decks_do_not_count() {
        let (mut store, deck, cards) = store_with_cards(25);
        let other = store.create_deck_at("Other", T0).unwrap().id;
        let other_card = store.add_card_at(&other, "q", "a", T0).unwrap().id;
        for card in &cards[..NEW_PER_DAY as usize] {
            answer(&mut store, card, Grade::Pass, T0);
        }
        let input = store.study_input(&deck, T0, DAY_START).unwrap();
        assert_eq!(input.new_today, NEW_PER_DAY);
        assert_eq!(input.new, Vec::<String>::new());
        let other_input = store.study_input(&other, T0, DAY_START).unwrap();
        assert_eq!(
            (other_input.new_today, other_input.new),
            (0, vec![other_card])
        );
        // The next day starts a new budget.
        let input = store
            .study_input(&deck, T0 + DAY_MS, DAY_START + DAY_MS)
            .unwrap();
        assert_eq!(input.new_today, 0);
        assert_eq!(input.new, cards[NEW_PER_DAY as usize..]);
    }

    #[test]
    fn reviews_are_due_now_and_steps_within_twenty_minutes() {
        let (mut store, deck, cards) = store_with_cards(4);
        // cards[0]: graduated long ago, due now.
        answer(&mut store, &cards[0], Grade::Pass, T0 - 60 * DAY_MS);
        answer(
            &mut store,
            &cards[0],
            Grade::Pass,
            T0 - 60 * DAY_MS + STEP_MS,
        );
        // cards[1]: in the step, due in 10 minutes.
        answer(&mut store, &cards[1], Grade::Fail, T0);
        // cards[2]: graduated just now, due in days.
        answer(&mut store, &cards[2], Grade::Pass, T0 - STEP_MS);
        answer(&mut store, &cards[2], Grade::Pass, T0);

        let input = store.study_input(&deck, T0, DAY_START).unwrap();
        assert_eq!(
            input.reviews.iter().map(|d| &d.card_id).collect::<Vec<_>>(),
            [&cards[0]]
        );
        assert_eq!(input.steps.len(), 1);
        assert_eq!(input.steps[0].card_id, cards[1]);
        assert_eq!(input.steps[0].due_at, T0 + STEP_MS);
        assert_eq!(input.new, [cards[3].clone()]);

        // A step card more than 20 minutes away is not in the input.
        let early = T0 + STEP_MS - EARLY_MS - 1;
        let input = store.study_input(&deck, early, DAY_START).unwrap();
        assert_eq!(input.steps, Vec::<Due>::new());
    }

    #[test]
    fn deleted_cards_are_never_offered() {
        let (mut store, deck, cards) = store_with_cards(2);
        answer(&mut store, &cards[0], Grade::Fail, T0);
        store.delete_card_at(&cards[0], T0).unwrap();
        store.delete_card_at(&cards[1], T0).unwrap();
        let input = store.study_input(&deck, T0, DAY_START).unwrap();
        assert!(input.is_empty());
        assert_eq!(input.new_today, 1);
    }

    #[test]
    fn a_whole_session_records_every_answer_and_keeps_the_cache_exact() {
        let (mut store, deck, cards) = store_with_cards(8);
        for card in &cards[..3] {
            answer(&mut store, card, Grade::Pass, T0 - 60 * DAY_MS);
            answer(&mut store, card, Grade::Pass, T0 - 60 * DAY_MS + STEP_MS);
        }
        let mut now = T0;
        let mut session = Session::new(store.study_input(&deck, now, DAY_START).unwrap());
        let mut answers = 0;
        while let Some(card) = session.next(now).map(str::to_owned) {
            // Fail every third answer; time passes 40 s per card.
            let grade = if answers % 3 == 2 {
                Grade::Fail
            } else {
                Grade::Pass
            };
            store
                .record_review(&card, grade, Duration::from_secs(40), now)
                .unwrap();
            let state = store.card_state(&card).unwrap();
            session.graded(&card, grade, Duration::from_secs(40), &state);
            answers += 1;
            now += 40_000;
            assert!(answers < 200, "the session does not end");
        }
        let summary = session.summary();
        assert_eq!(summary.cards, 8);
        assert_eq!(summary.answers, answers);
        let events: usize = cards
            .iter()
            .map(|c| store.review_events(c).unwrap().len())
            .sum();
        assert_eq!(events, 6 + answers, "every answer is in the log");
        for card in &cards {
            assert_eq!(store.card_state(card).unwrap(), rebuilt(&store, card));
            // Nothing is left in the step: every card ended in review.
            if let crate::domain::CardState::Scheduled(s) = store.card_state(card).unwrap() {
                assert_eq!(s.phase, Phase::Review);
            }
        }
        assert_eq!(
            store.study_input(&deck, now, DAY_START).unwrap().steps,
            Vec::<Due>::new()
        );
    }

    #[test]
    fn a_missing_or_deleted_deck_is_reported() {
        let (mut store, deck, _) = store_with_cards(1);
        assert!(matches!(
            store.study_input("01920000-0000-7000-8000-00000000dead", T0, DAY_START),
            Err(crate::Error::DeckNotFound(_))
        ));
        store.delete_deck_at(&deck, T0).unwrap();
        assert!(matches!(
            store.study_input(&deck, T0, DAY_START),
            Err(crate::Error::DeckNotFound(_))
        ));
    }

    #[test]
    fn reviews_and_steps_are_ordered_by_due_time_then_creation() {
        let (mut store, deck, cards) = store_with_cards(12);
        // Ten reviews with two different histories, answered in reverse
        // creation order: equal histories often give equal due times
        // (fuzz is per card), so creation order must break those ties.
        for (i, card) in cards[..10].iter().enumerate().rev() {
            let learned = T0 - if i % 2 == 0 { 80 } else { 60 } * DAY_MS;
            answer(&mut store, card, Grade::Pass, learned);
            answer(&mut store, card, Grade::Pass, learned + STEP_MS);
        }
        // Two step cards: cards[11] entered the step before cards[10].
        answer(&mut store, &cards[11], Grade::Fail, T0 - 2 * 60_000);
        answer(&mut store, &cards[10], Grade::Fail, T0 - 60_000);

        let input = store.study_input(&deck, T0, DAY_START).unwrap();
        let created = |id: &str| cards.iter().position(|c| c == id).unwrap();
        let keys: Vec<_> = input
            .reviews
            .iter()
            .map(|d| (d.due_at, created(&d.card_id)))
            .collect();
        let mut sorted = keys.clone();
        sorted.sort_unstable();
        assert_eq!(input.reviews.len(), 10);
        assert_eq!(keys, sorted, "most overdue first, then oldest card");
        let steps: Vec<_> = input.steps.iter().map(|d| d.card_id.clone()).collect();
        assert_eq!(steps, [cards[11].clone(), cards[10].clone()]);
    }
}

mod next_change {
    use std::time::Duration;

    use super::{T0, store_with_cards};
    use crate::Error;
    use crate::domain::{CardState, Grade};
    use crate::scheduling::session::EARLY_MS;

    fn due_at(store: &crate::storage::Store, card: &str) -> i64 {
        match store.card_state(card).unwrap() {
            CardState::Scheduled(s) => s.due_at,
            CardState::New => panic!("expected a schedule"),
        }
    }

    #[test]
    fn nothing_scheduled_never_changes() {
        let (store, deck, _) = store_with_cards(3);
        assert_eq!(store.next_study_change(&deck, T0).unwrap(), None);
    }

    #[test]
    fn a_step_card_counts_from_twenty_minutes_before_it_is_due() {
        let (mut store, deck, cards) = store_with_cards(2);
        store
            .record_review(&cards[0], Grade::Fail, Duration::ZERO, T0)
            .unwrap();
        let due = due_at(&store, &cards[0]);
        // Already within the early window at T0: offered now, not a change.
        assert_eq!(store.next_study_change(&deck, T0).unwrap(), None);
        // From before that window, the change is when it opens.
        let before = due - EARLY_MS - 1;
        assert_eq!(
            store.next_study_change(&deck, before).unwrap(),
            Some(due - EARLY_MS)
        );
        // And it is exactly when study_input starts to offer it.
        let at = |now| store.study_input(&deck, now, T0).unwrap().steps.len();
        assert_eq!((at(due - EARLY_MS - 1), at(due - EARLY_MS)), (0, 1));
    }

    #[test]
    fn a_review_card_counts_from_its_due_time_and_deleted_cards_never() {
        let (mut store, deck, cards) = store_with_cards(3);
        for card in &cards[..2] {
            store
                .record_review(card, Grade::Pass, Duration::ZERO, T0)
                .unwrap();
            let step_due = due_at(&store, card);
            store
                .record_review(card, Grade::Pass, Duration::ZERO, step_due)
                .unwrap();
        }
        let (first, second) = (due_at(&store, &cards[0]), due_at(&store, &cards[1]));
        let soonest = first.min(second);
        let now = soonest - 1;
        assert_eq!(store.next_study_change(&deck, now).unwrap(), Some(soonest));
        let reviews = |now| store.study_input(&deck, now, T0).unwrap().reviews.len();
        assert_eq!(reviews(soonest - 1), 0);
        assert!(reviews(soonest) >= 1);

        // The soonest card deleted: the other one is next.
        let soonest_card = if first <= second {
            &cards[0]
        } else {
            &cards[1]
        };
        store.delete_card_at(soonest_card, now).unwrap();
        assert_eq!(
            store.next_study_change(&deck, now).unwrap(),
            Some(first.max(second))
        );
    }

    /// A card due exactly at `now` is offered already, so the next change
    /// is a later one (`> now`, not `>= now`).
    #[test]
    fn a_card_due_exactly_now_is_not_the_next_change() {
        let (mut store, deck, cards) = store_with_cards(2);
        // An hour apart, so their due times differ.
        for (card, start) in cards.iter().zip([T0, T0 + 3_600_000]) {
            store
                .record_review(card, Grade::Pass, Duration::ZERO, start)
                .unwrap();
            let step_due = due_at(&store, card);
            store
                .record_review(card, Grade::Pass, Duration::ZERO, step_due)
                .unwrap();
        }
        let (a, b) = (due_at(&store, &cards[0]), due_at(&store, &cards[1]));
        assert_ne!(a, b, "the fixture needs two different due times");
        let (first, second) = (a.min(b), a.max(b));
        assert_eq!(store.next_study_change(&deck, first).unwrap(), Some(second));
        assert_eq!(store.next_study_change(&deck, second).unwrap(), None);
    }

    /// A review card failed goes to relearning: like a learning card, it
    /// counts from 20 minutes before its step is due.
    #[test]
    fn a_relearning_card_counts_from_twenty_minutes_before_it_is_due() {
        let (mut store, deck, cards) = store_with_cards(1);
        let card = &cards[0];
        store
            .record_review(card, Grade::Pass, Duration::ZERO, T0)
            .unwrap();
        let step_due = due_at(&store, card);
        store
            .record_review(card, Grade::Pass, Duration::ZERO, step_due)
            .unwrap();
        let review_due = due_at(&store, card);
        store
            .record_review(card, Grade::Fail, Duration::ZERO, review_due)
            .unwrap();
        match store.card_state(card).unwrap() {
            CardState::Scheduled(s) => assert_eq!(s.phase, crate::domain::Phase::Relearning),
            CardState::New => panic!("expected a schedule"),
        }
        let due = due_at(&store, card);
        let before = due - EARLY_MS - 1;
        assert_eq!(
            store.next_study_change(&deck, before).unwrap(),
            Some(due - EARLY_MS)
        );
        let steps = |now| store.study_input(&deck, now, T0).unwrap().steps.len();
        assert_eq!((steps(due - EARLY_MS - 1), steps(due - EARLY_MS)), (0, 1));
    }

    #[test]
    fn a_missing_deck_is_reported() {
        let (store, _, _) = store_with_cards(1);
        assert!(matches!(
            store.next_study_change("01920000-0000-7000-8000-00000000dead", T0),
            Err(Error::DeckNotFound(_))
        ));
    }
}

/// The daily limits from the settings (ADR 0027).
mod limits {
    use super::{DAY_MS, T0, store_with_cards};
    use crate::domain::Grade;
    use crate::scheduling::STEP_MS;
    use crate::scheduling::session::Due;
    use crate::storage::Store;
    use crate::test_support::answer;

    /// Study time, 40 days after the cards were learned: all are due.
    const NOW: i64 = T0 + 40 * DAY_MS;
    /// "Today" began an hour before `NOW`.
    const DAY_START: i64 = NOW - 3_600_000;

    /// A deck of `n` cards learned one day apart (the first is the most
    /// overdue at `NOW`), all in review.
    fn overdue(n: usize) -> (Store, String, Vec<String>) {
        let (mut store, deck, cards) = store_with_cards(n);
        for (i, card) in cards.iter().enumerate() {
            let at = T0 + i64::try_from(i).unwrap() * DAY_MS;
            answer(&mut store, card, Grade::Pass, at);
            answer(&mut store, card, Grade::Pass, at + STEP_MS);
        }
        (store, deck, cards)
    }

    fn reviews(store: &Store, deck: &str) -> Vec<String> {
        let input = store.study_input(deck, NOW, DAY_START).unwrap();
        input.reviews.into_iter().map(|d| d.card_id).collect()
    }

    #[test]
    fn the_new_card_limit_applies() {
        let (mut store, deck, cards) = store_with_cards(30);
        for (limit, shown) in [(0, 0), (1, 1), (20, 20), (30, 30), (9_999, 30)] {
            store.set_max_new_per_day(limit).unwrap();
            let input = store.study_input(&deck, T0, T0 - 1).unwrap();
            assert_eq!(input.new, cards[..shown], "limit {limit}");
        }
    }

    #[test]
    fn new_cards_already_started_today_use_the_limit() {
        let (mut store, deck, cards) = store_with_cards(10);
        store.set_max_new_per_day(3).unwrap();
        answer(&mut store, &cards[0], Grade::Pass, T0);
        let input = store.study_input(&deck, T0, T0 - 1).unwrap();
        assert_eq!((input.new_today, input.new.len()), (1, 2));
        // Raising the limit gives more at once; lowering it below what was
        // done gives none, not an underflow.
        store.set_max_new_per_day(6).unwrap();
        assert_eq!(store.study_input(&deck, T0, T0 - 1).unwrap().new.len(), 5);
        store.set_max_new_per_day(0).unwrap();
        assert_eq!(
            store.study_input(&deck, T0, T0 - 1).unwrap().new,
            Vec::<String>::new()
        );
    }

    #[test]
    fn the_review_limit_keeps_the_most_overdue() {
        let (mut store, deck, cards) = overdue(10);
        assert_eq!(reviews(&store, &deck), cards, "default limit shows all");
        for limit in [0, 1, 4, 10, 11] {
            store.set_max_reviews_per_day(limit).unwrap();
            let shown = reviews(&store, &deck);
            assert_eq!(shown, cards[..limit.min(10) as usize], "limit {limit}");
        }
    }

    #[test]
    fn reviews_done_today_use_the_review_limit() {
        let (mut store, deck, cards) = overdue(10);
        store.set_max_reviews_per_day(4).unwrap();
        answer(&mut store, &cards[0], Grade::Pass, NOW);
        let input = store.study_input(&deck, NOW, DAY_START).unwrap();
        assert_eq!(input.reviews_today, 1);
        let shown: Vec<_> = input.reviews.into_iter().map(|d| d.card_id).collect();
        assert_eq!(shown, cards[1..4], "3 left of 4");

        for card in &cards[1..4] {
            answer(&mut store, card, Grade::Pass, NOW);
        }
        let input = store.study_input(&deck, NOW, DAY_START).unwrap();
        assert_eq!((input.reviews_today, input.reviews.len()), (4, 0));
        // Lowering the limit below what was done is not an underflow.
        store.set_max_reviews_per_day(2).unwrap();
        assert_eq!(reviews(&store, &deck), Vec::<String>::new());
    }

    #[test]
    fn a_new_card_does_not_use_the_review_limit_and_a_review_not_the_new_one() {
        let (mut store, deck, cards) = overdue(3);
        let fresh = store.add_card_at(&deck, "fresh", "a", T0).unwrap().id;
        store.set_max_reviews_per_day(1).unwrap();
        store.set_max_new_per_day(1).unwrap();
        answer(&mut store, &fresh, Grade::Pass, NOW);
        answer(&mut store, &cards[0], Grade::Pass, NOW);
        let input = store.study_input(&deck, NOW, DAY_START).unwrap();
        assert_eq!((input.new_today, input.reviews_today), (1, 1));
        assert!(input.reviews.is_empty() && input.new.is_empty());
    }

    #[test]
    fn step_cards_are_not_limited() {
        let (mut store, deck, cards) = overdue(3);
        store.set_max_reviews_per_day(0).unwrap();
        store.set_max_new_per_day(0).unwrap();
        // Failed, so in the 10-minute step and due again within it.
        answer(&mut store, &cards[0], Grade::Fail, NOW);
        let input = store.study_input(&deck, NOW + STEP_MS, DAY_START).unwrap();
        assert_eq!(
            input.steps.iter().map(|d| &d.card_id).collect::<Vec<_>>(),
            [&cards[0]]
        );
        assert_eq!(input.reviews, Vec::<Due>::new());
    }

    #[test]
    fn deleted_cards_and_steps_that_cross_the_day_start_use_the_review_limit() {
        let (mut store, deck, cards) = overdue(4);
        store.set_max_reviews_per_day(2).unwrap();
        // Reviewed today, then deleted: still used today's budget.
        answer(&mut store, &cards[0], Grade::Pass, NOW);
        store.delete_card_at(&cards[0], NOW).unwrap();
        // Failed before the day start, its step answered after it: an older
        // card reviewed today, though step cards themselves are not held back.
        answer(&mut store, &cards[1], Grade::Fail, DAY_START - 60_000);
        answer(&mut store, &cards[1], Grade::Pass, NOW);
        let input = store.study_input(&deck, NOW, DAY_START).unwrap();
        assert_eq!(input.reviews_today, 2);
        assert_eq!(input.reviews, Vec::<Due>::new());
    }

    #[test]
    fn limits_are_per_deck() {
        let (mut store, deck, cards) = overdue(4);
        let other = store.create_deck_at("Other", T0).unwrap().id;
        let other_card = store.add_card_at(&other, "q", "a", T0).unwrap().id;
        store.set_max_reviews_per_day(2).unwrap();
        store.set_max_new_per_day(1).unwrap();
        for card in &cards[..2] {
            answer(&mut store, card, Grade::Pass, NOW);
        }
        assert_eq!(reviews(&store, &deck), Vec::<String>::new());
        let input = store.study_input(&other, NOW, DAY_START).unwrap();
        assert_eq!(input.new, [other_card]);
    }

    #[test]
    fn the_deck_list_counts_with_the_same_limits() {
        let (mut store, deck, _) = overdue(10);
        store.add_card_at(&deck, "fresh", "a", T0).unwrap();
        for (new, rev) in [(0, 0), (1, 3), (5, 10), (20, 200)] {
            store.set_max_new_per_day(new).unwrap();
            store.set_max_reviews_per_day(rev).unwrap();
            let summary = &store.list_decks(NOW, DAY_START).unwrap()[0];
            let input = store.study_input(&deck, NOW, DAY_START).unwrap();
            assert_eq!(
                (summary.new_count, summary.due_count),
                (new.min(1), rev.min(10)),
                "limits {new}/{rev}"
            );
            assert_eq!(
                (summary.new_count as usize, summary.due_count as usize),
                (input.new.len(), input.reviews.len() + input.steps.len())
            );
        }
    }

    #[test]
    fn the_day_start_decides_what_counts_as_today() {
        let (mut store, deck, cards) = overdue(3);
        // `cards[0]` was first reviewed at T0 and again at T0 + STEP_MS;
        // answer it once more at NOW, and a new card once at NOW.
        let fresh = store.add_card_at(&deck, "fresh", "a", T0).unwrap().id;
        answer(&mut store, &cards[0], Grade::Pass, NOW);
        answer(&mut store, &fresh, Grade::Pass, NOW);
        let counts = |day_start| {
            let input = store.study_input(&deck, NOW, day_start).unwrap();
            (input.new_today, input.reviews_today)
        };
        // The day began at the last review: both reviews count.
        assert_eq!(counts(NOW), (1, 1));
        // One millisecond later it has not begun yet: nothing counts.
        assert_eq!(counts(NOW + 1), (0, 0));
        // Began before NOW but after the older reviews.
        assert_eq!(counts(NOW - 1), (1, 1));
        // Began before every first review: all four cards were new that day.
        assert_eq!(counts(T0), (4, 0));
        // Began after the first card's first two reviews: it is a review
        // now, and the other two (learned a day or two later) and the
        // fresh card are new.
        assert_eq!(counts(T0 + STEP_MS + 1), (3, 1));
    }
}
