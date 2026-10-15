use proptest::prelude::*;

use super::*;
use crate::Error;
use crate::domain::{CardState, Schedule};
use crate::id::IdGen;
use crate::scheduling::{STEP_MS, Scheduler};

/// 2026-10-15T12:34:56.789Z
const T0: i64 = 1_792_067_696_789;
const DAY_MS: i64 = 86_400_000;

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

fn scheduled(state: CardState) -> Schedule {
    match state {
        CardState::Scheduled(s) => s,
        CardState::New => panic!("expected a scheduled card"),
    }
}

fn rebuilt(store: &Store, card: &str) -> CardState {
    Scheduler::default()
        .rebuild_state(card, &store.review_events(card).unwrap())
        .unwrap()
}

/// A `card_schedule` row: card id, phase, due, stability and difficulty as
/// bits, last review, reps, lapses.
type ScheduleRow = (String, String, i64, u64, u64, i64, i64, i64);

/// Every `card_schedule` row, with the floats as bits so equality is exact.
fn schedule_table(store: &Store) -> Vec<ScheduleRow> {
    let mut stmt = store
        .conn
        .prepare(
            "SELECT card_id, phase, due_at, stability, difficulty, last_review_at, reps, lapses
             FROM card_schedule ORDER BY card_id",
        )
        .unwrap();
    stmt.query_map([], |r| {
        Ok((
            r.get(0)?,
            r.get(1)?,
            r.get(2)?,
            r.get::<_, f64>(3)?.to_bits(),
            r.get::<_, f64>(4)?.to_bits(),
            r.get(5)?,
            r.get(6)?,
            r.get(7)?,
        ))
    })
    .unwrap()
    .collect::<rusqlite::Result<_>>()
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
    assert_eq!(store.list_decks(T0).unwrap().len(), 2);
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
        card: usize,
        pass: bool,
        clock_delta: i64,
        duration_ms: u64,
    },
    /// A new process: the id generator starts from scratch.
    Restart,
}

fn step() -> impl Strategy<Value = Step> {
    let delta = prop_oneof![
        3 => Just(0i64),
        2 => -2 * DAY_MS..0,
        3 => 0..=3 * STEP_MS,
        3 => 0..=60 * DAY_MS,
    ];
    let review = (0usize..4, any::<bool>(), delta, 0u64..120_000).prop_map(
        |(card, pass, clock_delta, duration_ms)| Step::Review {
            card,
            pass,
            clock_delta,
            duration_ms,
        },
    );
    prop_oneof![9 => review, 1 => Just(Step::Restart)]
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(256))]

    /// The cache written review by review equals a full rebuild from the
    /// log, whatever the clock does and however often the app restarts.
    #[test]
    fn cache_equals_rebuild_after_random_reviews(
        n_cards in 1usize..=4,
        steps in prop::collection::vec(step(), 1..200),
    ) {
        let (mut store, _, cards) = store_with_cards(n_cards);
        let mut clock = T0;
        let mut recorded: Vec<Vec<ReviewEvent>> = vec![Vec::new(); n_cards];
        for step in steps {
            match step {
                Step::Restart => store.ids = IdGen::new(),
                Step::Review { card, pass, clock_delta, duration_ms } => {
                    clock += clock_delta;
                    let card = card % n_cards;
                    let grade = if pass { Grade::Pass } else { Grade::Fail };
                    let e = store
                        .record_review(&cards[card], grade, Duration::from_millis(duration_ms), clock)
                        .unwrap();
                    let floor = recorded[card].last().map_or(clock, |l| l.reviewed_at.max(clock));
                    prop_assert_eq!(e.reviewed_at, floor);
                    recorded[card].push(e);
                }
            }
        }

        for (card, events) in cards.iter().zip(&recorded) {
            // (c) The log replays in the order it was recorded.
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
    use crate::scheduling::session::{Due, EARLY_MS, NEW_PER_DAY, Session};
    use crate::storage::Store;

    /// "Today" began two hours before `T0`.
    const DAY_START: i64 = T0 - 2 * 3_600_000;

    fn answer(store: &mut Store, card: &str, grade: Grade, at: i64) {
        store
            .record_review(card, grade, Duration::from_secs(3), at)
            .unwrap();
    }

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
