use fsrs::{FSRSItem, FSRSReview};
use proptest::prelude::*;

use super::fuzz::{MAX_INTERVAL_DAYS, floored_range, fuzz_factor, fuzz_range, fuzzed_days};
use super::*;

const CARD: &str = "01929d4e-7a3c-7b12-8f00-0123456789ab";
/// 2026-10-15 12:34:56.789 UTC.
const T0: i64 = 1_792_067_696_789;
const HOUR_MS: i64 = 3_600_000;

fn scheduled(state: CardState) -> Schedule {
    match state {
        CardState::Scheduled(s) => s,
        CardState::New => panic!("expected a scheduled card"),
    }
}

/// Applies `grades` at the given times, one `schedule` call per review.
fn run(s: &Scheduler, reviews: &[(i64, Grade)]) -> CardState {
    reviews.iter().fold(CardState::New, |state, &(at, g)| {
        s.schedule(CARD, &state, g, at).unwrap()
    })
}

fn event(i: usize, at: i64, grade: Grade) -> ReviewEvent {
    ReviewEvent {
        id: format!("01929d4e-0000-7000-8000-{i:012}"),
        card_id: CARD.to_owned(),
        reviewed_at: at,
        grade,
        duration_ms: 1000,
    }
}

// --- Reference values from the fsrs crate's own tests (fsrs 6.6.2) ---

/// `FSRS::next_states` doc-test: a new card with default parameters.
#[test]
fn first_review_matches_fsrs_reference() {
    let s = Scheduler::default();
    let fail = scheduled(s.schedule(CARD, &CardState::New, Grade::Fail, T0).unwrap());
    assert_eq!((fail.stability, fail.difficulty), (0.212, 6.4133));
    let pass = scheduled(s.schedule(CARD, &CardState::New, Grade::Pass, T0).unwrap());
    assert_eq!((pass.stability, pass.difficulty), (2.3065, 2.118_104));
}

/// `inference::tests::test_memory_state`: ratings 1,3,3,3,3,3 at elapsed
/// days 0,0,1,3,8,21 with default parameters.
#[test]
fn review_sequence_matches_fsrs_reference() {
    let s = Scheduler::default();
    let mut at = T0;
    let mut reviews = vec![(at, Grade::Fail)];
    at += STEP_MS; // same day: elapsed 0
    reviews.push((at, Grade::Pass));
    for days in [1, 3, 8, 21] {
        at += days * DAY_MS;
        reviews.push((at, Grade::Pass));
    }
    let end = scheduled(run(&s, &reviews));
    assert!(
        (end.stability - 53.626_91).abs() < 1e-4,
        "{}",
        end.stability
    );
    assert!(
        (end.difficulty - 6.357_486_7).abs() < 1e-4,
        "{}",
        end.difficulty
    );
    assert_eq!((end.reps, end.lapses, end.phase), (6, 0, Phase::Review));
}

/// `inference::tests::test_next_interval`: stability 1 day.
#[test]
fn interval_matches_fsrs_reference() {
    let days = |r: f32| Scheduler::new(r).unwrap().interval(1.0).round().max(1.0);
    assert_eq!(days(0.9), 1.0);
    assert_eq!(days(0.8), 3.0);
}

// --- State machine ---

#[test]
fn new_card_always_gets_the_ten_minute_step() {
    let s = Scheduler::default();
    for g in [Grade::Fail, Grade::Pass] {
        let st = scheduled(s.schedule(CARD, &CardState::New, g, T0).unwrap());
        assert_eq!(st.phase, Phase::Learning);
        assert_eq!(st.due_at, T0 + STEP_MS);
        assert_eq!((st.reps, st.lapses, st.last_review_at), (1, 0, T0));
    }
}

#[test]
fn learning_fail_repeats_the_step_and_pass_graduates() {
    let s = Scheduler::default();
    let learning = run(&s, &[(T0, Grade::Fail)]);

    let at = T0 + STEP_MS;
    let again = scheduled(s.schedule(CARD, &learning, Grade::Fail, at).unwrap());
    assert_eq!(
        (again.phase, again.due_at, again.lapses),
        (Phase::Learning, at + STEP_MS, 0)
    );

    let pass = scheduled(s.schedule(CARD, &learning, Grade::Pass, at).unwrap());
    assert_eq!(pass.phase, Phase::Review);
    let days = (pass.due_at - at) / DAY_MS;
    assert_eq!((pass.due_at - at) % DAY_MS, 0);
    assert!(days >= 1);
}

#[test]
fn review_fail_relearns_and_counts_a_lapse() {
    let s = Scheduler::default();
    let review = run(&s, &[(T0, Grade::Pass), (T0 + STEP_MS, Grade::Pass)]);
    let at = scheduled(review).due_at;

    let lapse = scheduled(s.schedule(CARD, &review, Grade::Fail, at).unwrap());
    assert_eq!(
        (lapse.phase, lapse.due_at, lapse.lapses),
        (Phase::Relearning, at + STEP_MS, 1)
    );
    assert!(lapse.stability < scheduled(review).stability);

    let at2 = at + STEP_MS;
    let state = CardState::Scheduled(lapse);
    let again = scheduled(s.schedule(CARD, &state, Grade::Fail, at2).unwrap());
    assert_eq!(
        (again.phase, again.lapses),
        (Phase::Relearning, 1),
        "no lapse in relearning"
    );
    let back = scheduled(s.schedule(CARD, &state, Grade::Pass, at2).unwrap());
    assert_eq!((back.phase, back.lapses, back.reps), (Phase::Review, 1, 4));
}

#[test]
fn review_pass_grows_the_interval() {
    let s = Scheduler::default();
    let mut state = run(&s, &[(T0, Grade::Pass), (T0 + STEP_MS, Grade::Pass)]);
    let mut last_interval = 0;
    for _ in 0..6 {
        let st = scheduled(state);
        let interval = st.due_at - st.last_review_at;
        assert!(interval > last_interval, "{interval} <= {last_interval}");
        last_interval = interval;
        state = s.schedule(CARD, &state, Grade::Pass, st.due_at).unwrap();
    }
}

#[test]
fn elapsed_days_round_to_the_nearest_day() {
    assert_eq!(elapsed_days(T0, T0), 0);
    assert_eq!(elapsed_days(T0, T0 + 11 * HOUR_MS), 0);
    assert_eq!(elapsed_days(T0, T0 + 12 * HOUR_MS - 1), 0);
    assert_eq!(elapsed_days(T0, T0 + 12 * HOUR_MS), 1);
    assert_eq!(elapsed_days(T0, T0 + 13 * HOUR_MS), 1);
    assert_eq!(elapsed_days(T0, T0 + 36 * HOUR_MS), 2);
    assert_eq!(elapsed_days(T0, T0 - DAY_MS), 0, "clock went backwards");
    assert_eq!(elapsed_days(i64::MIN, i64::MAX), u32::MAX);
}

/// A review dated before the last one would be replayed in a different
/// order (rebuild sorts by time), so `schedule` refuses it. Recording
/// code clamps to `max(now, last_review_at)`, which is accepted.
#[test]
fn review_before_the_last_review_is_refused() {
    let s = Scheduler::default();
    let state = run(&s, &[(T0, Grade::Pass)]);
    let err = s.schedule(CARD, &state, Grade::Pass, T0 - 1);
    assert!(
        matches!(
            err,
            Err(Error::ReviewBeforeLastReview {
                reviewed_at,
                last_review_at: T0
            }) if reviewed_at == T0 - 1
        ),
        "{err:?}"
    );
    assert!(s.schedule(CARD, &state, Grade::Pass, T0).is_ok());
}

#[test]
fn desired_retention_is_validated() {
    assert!(Scheduler::new(0.80).is_ok());
    assert!(Scheduler::new(0.97).is_ok());
    assert_eq!(Scheduler::new(0.90).unwrap().desired_retention(), 0.90);
    for bad in [0.79, 0.98, 0.0, 1.0, f32::NAN] {
        assert!(
            matches!(Scheduler::new(bad), Err(Error::InvalidRetention(_))),
            "{bad}"
        );
    }
    assert_eq!(
        Scheduler::default().desired_retention(),
        DEFAULT_DESIRED_RETENTION
    );
}

#[test]
fn higher_retention_gives_shorter_intervals() {
    let reviews = [(T0, Grade::Pass), (T0 + STEP_MS, Grade::Pass)];
    let interval = |r| {
        let st = scheduled(run(&Scheduler::new(r).unwrap(), &reviews));
        st.due_at - st.last_review_at
    };
    assert!(interval(0.97) < interval(0.80));
}

#[test]
fn non_finite_state_is_an_error() {
    let bad = CardState::Scheduled(Schedule {
        phase: Phase::Review,
        stability: f32::NAN,
        difficulty: 5.0,
        due_at: T0,
        last_review_at: T0,
        reps: 1,
        lapses: 0,
    });
    let err = Scheduler::default().schedule(CARD, &bad, Grade::Pass, T0 + DAY_MS);
    assert!(matches!(err, Err(Error::Fsrs(_))), "{err:?}");
}

#[test]
fn rebuild_of_no_events_is_new() {
    assert_eq!(
        Scheduler::default().rebuild_state(CARD, &[]).unwrap(),
        CardState::New
    );
}

#[test]
fn rebuild_rejects_events_of_another_card() {
    let mut e = event(0, T0, Grade::Pass);
    e.card_id = "01929d4e-7a3c-7b12-8f00-ffffffffffff".into();
    let err = Scheduler::default().rebuild_state(CARD, &[event(1, T0, Grade::Pass), e]);
    assert!(
        matches!(err, Err(Error::EventForOtherCard { .. })),
        "{err:?}"
    );
}

#[test]
fn rebuild_orders_equal_timestamps_by_id() {
    let s = Scheduler::default();
    let a = event(1, T0, Grade::Fail);
    let b = event(2, T0, Grade::Pass);
    let expected = run(&s, &[(T0, Grade::Fail), (T0, Grade::Pass)]);
    assert_eq!(
        s.rebuild_state(CARD, &[b.clone(), a.clone()]).unwrap(),
        expected
    );
    assert_eq!(s.rebuild_state(CARD, &[a, b]).unwrap(), expected);
}

#[test]
fn fuzz_depends_on_card_and_review_count() {
    // A long-lived card: Pass every time it is due. Its due dates must be
    // identical when replayed, and fuzz must actually move some of them
    // compared with another card id.
    let s = Scheduler::default();
    let walk = |card: &str| {
        let mut state = CardState::New;
        let mut at = T0;
        let mut dues = vec![];
        for _ in 0..12 {
            state = s.schedule(card, &state, Grade::Pass, at).unwrap();
            at = scheduled(state).due_at;
            dues.push(at);
        }
        dues
    };
    assert_eq!(walk(CARD), walk(CARD));
    assert_ne!(walk(CARD), walk("01929d4e-7a3c-7b12-8f00-000000000001"));
}

/// A review card recalled long after it was due, whose new FSRS interval
/// is shorter than the gap it survived: the Pass is scheduled at least
/// elapsed + 1 days out, whatever the fuzz (ADR 0009).
#[test]
fn late_review_pass_is_at_least_elapsed_plus_one_day() {
    let s = Scheduler::default();
    let hard = Schedule {
        phase: Phase::Review,
        stability: 0.5,
        difficulty: 10.0,
        due_at: T0 + DAY_MS,
        last_review_at: T0,
        reps: 5,
        lapses: 3,
    };
    let elapsed = 30;
    let now = T0 + i64::from(elapsed) * DAY_MS;
    for i in 0..500 {
        let card = format!("01929d4e-7a3c-7b12-8f00-{i:012x}");
        let st = scheduled(
            s.schedule(&card, &CardState::Scheduled(hard), Grade::Pass, now)
                .unwrap(),
        );
        // The floor matters here: FSRS alone would give less.
        assert!(f64::from(s.interval(st.stability)) < f64::from(elapsed + 1));
        let days = (st.due_at - now) / DAY_MS;
        assert!(days > i64::from(elapsed), "{card}: {days} days");
        let (lo, hi) = floored_range(f64::from(s.interval(st.stability)), elapsed + 1);
        assert!((i64::from(lo)..=i64::from(hi)).contains(&days));
    }
}

/// A Pass on time, where FSRS's interval is above the floor, is not
/// affected by it.
#[test]
fn on_time_review_pass_ignores_the_floor() {
    let s = Scheduler::default();
    let mut state = run(&s, &[(T0, Grade::Pass), (T0 + STEP_MS, Grade::Pass)]);
    for _ in 0..4 {
        let st = scheduled(state);
        state = s.schedule(CARD, &state, Grade::Pass, st.due_at).unwrap();
    }
    let prev = scheduled(state);
    let now = prev.due_at;
    let next = scheduled(s.schedule(CARD, &state, Grade::Pass, now).unwrap());
    let elapsed = elapsed_days(prev.last_review_at, now);
    let interval = f64::from(s.interval(next.stability));
    assert!(interval > f64::from(elapsed + 1) + 10.0, "{interval}");
    assert_eq!(floored_range(interval, elapsed + 1), fuzz_range(interval));
    let days = u32::try_from((next.due_at - now) / DAY_MS).unwrap();
    assert_eq!(days, fuzzed_days(interval, fuzz_factor(CARD, prev.reps)));
}

// --- Property tests ---

/// Time between two reviews: mostly same-session gaps or up to 400 days,
/// rarely up to 110 years (beyond the 100-year interval cap).
fn gap() -> impl Strategy<Value = i64> {
    prop_oneof![
        10 => 0..3 * STEP_MS,
        10 => 0..2 * DAY_MS,
        10 => 0..400 * DAY_MS,
        1 => 0..40_000 * DAY_MS,
    ]
}

fn grade() -> impl Strategy<Value = Grade> {
    prop_oneof![Just(Grade::Fail), Just(Grade::Pass)]
}

fn card_id() -> impl Strategy<Value = String> {
    "[0-9a-f]{8}-[0-9a-f]{4}-7[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}"
}

fn history() -> impl Strategy<Value = (String, Vec<ReviewEvent>, f32)> {
    (
        card_id(),
        prop::collection::vec((gap(), grade()), 1..40),
        prop_oneof![Just(0.80_f32), Just(0.90), Just(0.97)],
    )
        .prop_map(|(card, steps, retention)| {
            let mut at = T0;
            let events = steps
                .into_iter()
                .enumerate()
                .map(|(i, (gap, grade))| {
                    at += gap;
                    ReviewEvent {
                        card_id: card.clone(),
                        ..event(i, at, grade)
                    }
                })
                .collect();
            (card, events, retention)
        })
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(512))]

    /// The core guarantee: rebuilding from the log gives exactly the state
    /// reached by scheduling review by review, and every intermediate
    /// state satisfies the scheduler's invariants.
    #[test]
    fn rebuild_equals_reviewing_one_by_one((card, events, retention) in history()) {
        let s = Scheduler::new(retention).unwrap();
        let mut state = CardState::New;
        let mut lapses = 0;
        let mut fsrs_reviews = vec![];
        for (n, e) in events.iter().enumerate() {
            let prev = state;
            state = s.schedule(&card, &prev, e.grade, e.reviewed_at).unwrap();
            let st = scheduled(state);

            // Counters.
            if let CardState::Scheduled(p) = prev
                && p.phase == Phase::Review
                && e.grade == Grade::Fail
            {
                lapses += 1;
            }
            prop_assert_eq!(st.reps as usize, n + 1);
            prop_assert_eq!(st.lapses, lapses);
            prop_assert_eq!(st.last_review_at, e.reviewed_at);

            // Memory state is finite and inside FSRS's bounds.
            prop_assert!((0.001..=36_500.0).contains(&st.stability), "S = {}", st.stability);
            prop_assert!((1.0..=10.0).contains(&st.difficulty), "D = {}", st.difficulty);

            // Due date: the 10-minute step, or whole days within the fuzz
            // range of FSRS's interval raised to the floor (ADR 0009). A
            // Pass on a review card is never shorter than the gap + 1 day.
            let wait = st.due_at - e.reviewed_at;
            let (prev_phase, elapsed) = match prev {
                CardState::New => (None, 0),
                CardState::Scheduled(p) => (Some(p.phase), elapsed_days(p.last_review_at, e.reviewed_at)),
            };
            match st.phase {
                Phase::Learning | Phase::Relearning => prop_assert_eq!(wait, STEP_MS),
                Phase::Review => {
                    prop_assert_eq!(wait % DAY_MS, 0);
                    let days = u32::try_from(wait / DAY_MS).unwrap();
                    let floor = prev_phase.map_or(1, |p| pass_floor(p, elapsed));
                    let (lo, hi) = floored_range(f64::from(s.interval(st.stability)), floor);
                    prop_assert!((lo..=hi).contains(&days), "{days} not in {lo}..={hi}");
                    prop_assert!((1..=MAX_INTERVAL_DAYS).contains(&days));
                    if prev_phase == Some(Phase::Review) {
                        let min = elapsed.saturating_add(1).min(MAX_INTERVAL_DAYS);
                        prop_assert!(days >= min, "{days} < {min}");
                    }
                }
            }

            // Independent oracle: fsrs's own replay of the same ratings.
            let delta_t = match prev {
                CardState::New => 0,
                CardState::Scheduled(p) => elapsed_days(p.last_review_at, e.reviewed_at),
            };
            let rating = match e.grade { Grade::Fail => 1, Grade::Pass => 3 };
            fsrs_reviews.push(FSRSReview { rating, delta_t });
            let expected = fsrs::FSRS::default()
                .memory_state(FSRSItem { reviews: fsrs_reviews.clone() }, None)
                .unwrap();
            prop_assert_eq!((st.stability, st.difficulty), (expected.stability, expected.difficulty));
        }
        prop_assert_eq!(s.rebuild_state(&card, &events).unwrap(), state);
    }

    /// Whatever the history, a review dated before the last one is refused,
    /// so no sequence of successful `schedule` calls can produce a log
    /// that rebuilds differently.
    #[test]
    fn review_going_back_in_time_is_refused(
        (card, events, retention) in history(),
        back in 1..400 * DAY_MS,
        grade in grade(),
    ) {
        let s = Scheduler::new(retention).unwrap();
        let state = s.rebuild_state(&card, &events).unwrap();
        let last = scheduled(state).last_review_at;
        let result = s.schedule(&card, &state, grade, last - back);
        prop_assert!(
            matches!(result, Err(Error::ReviewBeforeLastReview { .. })),
            "{:?}",
            result
        );
    }

    /// The order of the slice does not matter, only timestamps and ids.
    #[test]
    fn rebuild_ignores_input_order(
        (card, events, retention) in history().prop_flat_map(|(c, e, r)| {
            (Just(c), Just(e).prop_shuffle(), Just(r))
        })
    ) {
        let s = Scheduler::new(retention).unwrap();
        let mut sorted = events.clone();
        sorted.sort_by(|a, b| (a.reviewed_at, &a.id).cmp(&(b.reviewed_at, &b.id)));
        let one_by_one = sorted.iter().fold(CardState::New, |st, e| {
            s.schedule(&card, &st, e.grade, e.reviewed_at).unwrap()
        });
        prop_assert_eq!(s.rebuild_state(&card, &events).unwrap(), one_by_one);
    }
}
