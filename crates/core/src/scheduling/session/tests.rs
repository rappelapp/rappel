use proptest::prelude::*;

use super::*;
use crate::domain::Schedule;
use crate::test_support::T0;

const MIN: i64 = 60 * 1000;

fn due(id: &str, due_at: i64) -> Due {
    Due {
        card_id: id.to_owned(),
        due_at,
    }
}

fn ids(names: &[&str]) -> Vec<String> {
    names.iter().map(|&n| n.to_owned()).collect()
}

fn state(phase: Phase, due_at: i64) -> CardState {
    CardState::Scheduled(Schedule {
        phase,
        stability: 1.0,
        difficulty: 5.0,
        due_at,
        last_review_at: T0,
        reps: 1,
        lapses: 0,
    })
}

/// Shows and answers cards at `now` until the session is over, answering
/// each with `answer` and the state it returns. Returns the cards shown.
fn run(
    session: &mut Session,
    now: i64,
    mut answer: impl FnMut(&str) -> (Grade, CardState),
) -> Vec<String> {
    let mut shown = Vec::new();
    while let Some(id) = session.next(now).map(str::to_owned) {
        assert!(shown.len() < 1_000, "the session does not end");
        let (grade, state) = answer(&id);
        session.graded(&id, grade, Duration::from_secs(1), &state);
        shown.push(id);
    }
    shown
}

#[test]
fn interleave_spreads_new_cards_evenly() {
    let r = ids(&["r1", "r2", "r3", "r4", "r5", "r6"]);
    let n = ids(&["n1", "n2"]);
    assert_eq!(
        interleave(r, n),
        ids(&["r1", "r2", "n1", "r3", "r4", "n2", "r5", "r6"])
    );
    assert_eq!(interleave(ids(&["r"]), ids(&["n"])), ids(&["r", "n"]));
    assert_eq!(
        interleave(Vec::new(), ids(&["n1", "n2"])),
        ids(&["n1", "n2"])
    );
    assert_eq!(
        interleave(ids(&["r1", "r2"]), Vec::new()),
        ids(&["r1", "r2"])
    );
    assert_eq!(
        interleave::<String>(Vec::new(), Vec::new()),
        Vec::<String>::new()
    );
}

proptest! {
    #[test]
    fn interleave_keeps_everything_in_order(r in 0_usize..60, n in 0_usize..60) {
        let reviews: Vec<(bool, usize)> = (0..r).map(|j| (false, j)).collect();
        let new: Vec<(bool, usize)> = (0..n).map(|i| (true, i)).collect();
        let out = interleave(reviews.clone(), new.clone());
        prop_assert_eq!(out.len(), r + n);
        let out_reviews: Vec<_> = out.iter().copied().filter(|x| !x.0).collect();
        let out_new: Vec<_> = out.iter().copied().filter(|x| x.0).collect();
        prop_assert_eq!(out_reviews, reviews);
        prop_assert_eq!(out_new, new);
        if r >= n {
            prop_assert!(
                out.windows(2).all(|w| !(w[0].0 && w[1].0)),
                "two new cards side by side"
            );
        }
    }
}

#[test]
fn allowance_and_cap() {
    assert_eq!(allowance(20, 0), 20);
    assert_eq!(allowance(20, 5), 15);
    assert_eq!(allowance(20, 20), 0);
    assert_eq!(allowance(20, 23), 0);
    assert_eq!(allowance(0, 0), 0);
    assert_eq!(capped(Duration::from_secs(5)), Duration::from_secs(5));
    assert_eq!(capped(Duration::from_secs(3_600)), MAX_DURATION);
}

#[test]
fn due_steps_come_first_then_the_queue() {
    let input = StudyInput {
        reviews: vec![due("r1", T0 - MIN), due("r2", T0)],
        steps: vec![due("s-later", T0 + 5 * MIN), due("s-due", T0 - MIN)],
        new: ids(&["n1"]),
        new_today: 0,
        reviews_today: 0,
    };
    let mut session = Session::new(input);
    assert_eq!(session.remaining(), 5);
    let shown = run(&mut session, T0, |_| {
        (Grade::Pass, state(Phase::Review, T0 + 9 * 86_400_000))
    });
    // s-later is shown early, only once the queue is empty.
    assert_eq!(shown, ids(&["s-due", "r1", "n1", "r2", "s-later"]));
    assert_eq!(session.remaining(), 0);
}

#[test]
fn a_card_in_the_step_returns_in_the_same_session() {
    let input = StudyInput {
        new: ids(&["n1", "n2"]),
        ..StudyInput::default()
    };
    let mut session = Session::new(input);
    let mut answers = 0;
    let shown = run(&mut session, T0, |_| {
        answers += 1;
        // New cards enter the 10-minute step; their second answer
        // graduates them.
        if answers <= 2 {
            (Grade::Pass, state(Phase::Learning, T0 + 10 * MIN))
        } else {
            (Grade::Pass, state(Phase::Review, T0 + 86_400_000))
        }
    });
    assert_eq!(shown, ids(&["n1", "n2", "n1", "n2"]));
    let summary = session.summary();
    assert_eq!((summary.cards, summary.answers, summary.passes), (2, 4, 4));
}

#[test]
fn early_showing_is_at_most_twenty_minutes_and_otherwise_ends() {
    let input = StudyInput {
        steps: vec![due("far", T0 + EARLY_MS + 1), due("near", T0 + EARLY_MS)],
        ..StudyInput::default()
    };
    let mut session = Session::new(input);
    assert_eq!(session.next(T0), Some("near"));
    session.graded(
        "near",
        Grade::Pass,
        Duration::ZERO,
        &state(Phase::Review, T0 + 86_400_000),
    );
    // Only a card more than 20 minutes away is left: the session is over,
    // and the card stays for a later session.
    assert_eq!(session.next(T0), None);
    assert_eq!(session.remaining(), 1);
    // A millisecond later it is within the window.
    assert_eq!(session.next(T0 + 1), Some("far"));
}

#[test]
fn a_failed_review_relearns_in_the_session() {
    let input = StudyInput {
        reviews: vec![due("r1", T0)],
        ..StudyInput::default()
    };
    let mut session = Session::new(input);
    session.graded(
        "r1",
        Grade::Fail,
        Duration::ZERO,
        &state(Phase::Relearning, T0 + 10 * MIN),
    );
    assert_eq!(
        session.next(T0),
        Some("r1"),
        "shown early: nothing else left"
    );
    session.graded(
        "r1",
        Grade::Pass,
        Duration::ZERO,
        &state(Phase::Review, T0 + 86_400_000),
    );
    assert_eq!(session.next(T0), None);
}

#[test]
fn summary_counts_answers_passes_and_capped_time() {
    let mut session = Session::new(StudyInput {
        reviews: vec![due("a", T0), due("b", T0), due("c", T0)],
        ..StudyInput::default()
    });
    let review = state(Phase::Review, T0 + 86_400_000);
    session.graded("a", Grade::Pass, Duration::from_secs(10), &review);
    session.graded(
        "b",
        Grade::Fail,
        Duration::from_secs(600),
        &state(Phase::Relearning, T0),
    );
    session.graded("b", Grade::Pass, Duration::from_millis(2_500), &review);
    let summary = session.summary();
    assert_eq!(summary.cards, 2);
    assert_eq!(summary.answers, 3);
    assert_eq!(summary.passes, 2);
    assert_eq!(summary.time, Duration::from_millis(72_500));
    assert_eq!(summary.pass_percent(), Some(67));
    assert_eq!(Summary::default().pass_percent(), None);
    assert_eq!(session.next(T0), Some("c"));
}

#[test]
fn empty_input_is_empty() {
    assert!(StudyInput::default().is_empty());
    let input = StudyInput {
        new: ids(&["n"]),
        ..StudyInput::default()
    };
    assert!(!input.is_empty());
    assert_eq!(Session::new(StudyInput::default()).next(T0), None);
}

#[test]
fn a_skipped_card_is_dropped_and_not_counted() {
    let mut session = Session::new(StudyInput {
        reviews: vec![due("a", T0), due("b", T0)],
        steps: vec![due("s", T0 - MIN)],
        ..StudyInput::default()
    });
    session.skip("s");
    session.skip("a");
    assert_eq!(session.next(T0), Some("b"));
    assert_eq!(session.remaining(), 1);
    assert_eq!(session.summary(), Summary::default());
}
