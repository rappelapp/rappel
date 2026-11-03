//! Practice: its order, its in-memory answers and their undo (ADR 0070).

use std::time::Duration;

use proptest::prelude::*;

use super::*;

fn reviewed(id: &str, r: f32) -> PracticeCandidate {
    PracticeCandidate {
        card_id: id.to_owned(),
        seq: 0,
        retrievability: Some(r),
    }
}

fn new_card(id: &str, seq: i64) -> PracticeCandidate {
    PracticeCandidate {
        card_id: id.to_owned(),
        seq,
        retrievability: None,
    }
}

fn ids(names: &[&str]) -> Vec<String> {
    names.iter().map(|&n| n.to_owned()).collect()
}

const SECOND: Duration = Duration::from_secs(1);

#[test]
fn the_least_remembered_come_first_then_the_new_cards_oldest_first() {
    let order = practice_order(vec![
        new_card("n-late", 9),
        reviewed("high", 0.97),
        new_card("n-early", 2),
        reviewed("low", 0.41),
        reviewed("mid", 0.80),
    ]);
    assert_eq!(order, ids(&["low", "mid", "high", "n-early", "n-late"]));
}

#[test]
fn ties_break_by_id_whatever_the_input_order() {
    let a = vec![
        reviewed("b", 0.5),
        reviewed("a", 0.5),
        new_card("y", 3),
        new_card("x", 3),
    ];
    let mut b = a.clone();
    b.reverse();
    let want = ids(&["a", "b", "x", "y"]);
    assert_eq!(practice_order(a), want);
    assert_eq!(practice_order(b), want);
}

#[test]
fn a_practice_is_cut_to_the_least_remembered_cards() {
    let mut cards: Vec<PracticeCandidate> = (0..PRACTICE_LIMIT + 10)
        .map(|i| new_card(&format!("n{i:03}"), i64::try_from(i).unwrap()))
        .collect();
    #[expect(
        clippy::cast_precision_loss,
        reason = "small test numbers, exact in f32"
    )]
    cards.extend((0..20).map(|i| reviewed(&format!("r{i:02}"), 0.9 - i as f32 / 100.0)));
    let order = practice_order(cards);
    assert_eq!(order.len(), PRACTICE_LIMIT);
    // Every reviewed card, the lowest first, then the oldest new ones.
    assert_eq!(order[0], "r19");
    assert_eq!(order[19], "r00");
    assert_eq!(order[20], "n000");
    assert_eq!(
        order[PRACTICE_LIMIT - 1],
        format!("n{:03}", PRACTICE_LIMIT - 21)
    );
    assert_eq!(practice_order(Vec::new()), Vec::<String>::new());
}

#[test]
fn a_pass_ends_a_card_and_a_fail_brings_it_back_a_few_cards_later() {
    let mut p = Practice::new(ids(&["a", "b", "c", "d", "e", "f"]));
    assert_eq!(p.next(), Some("a"));
    assert_eq!(p.remaining(), 6);
    p.graded("a", Grade::Fail, SECOND);
    // After RELEARN_GAP = 3 other cards.
    assert_eq!(
        p.queue,
        VecDeque::from(ids(&["b", "c", "d", "a", "e", "f"]))
    );
    p.graded("b", Grade::Pass, SECOND);
    assert_eq!(p.next(), Some("c"));
    assert_eq!(p.remaining(), 5);
    let s = p.summary();
    assert_eq!((s.cards, s.answers, s.passes), (2, 2, 1));
    assert_eq!(s.pass_percent(), Some(50));
}

#[test]
fn a_failed_card_near_the_end_comes_back_at_the_end_and_alone_at_once() {
    let mut p = Practice::new(ids(&["a", "b"]));
    p.graded("a", Grade::Fail, SECOND);
    assert_eq!(p.queue, VecDeque::from(ids(&["b", "a"])));
    p.graded("b", Grade::Pass, SECOND);
    assert_eq!(p.next(), Some("a"));
    p.graded("a", Grade::Fail, SECOND);
    assert_eq!(p.next(), Some("a"));
    p.graded("a", Grade::Pass, SECOND);
    assert_eq!(p.next(), None);
    // Two distinct cards, four answers, two of them passes.
    let s = p.summary();
    assert_eq!((s.cards, s.answers, s.passes), (2, 4, 2));
}

#[test]
fn undo_steps_back_to_the_card_and_out_of_the_summary() {
    let mut p = Practice::new(ids(&["a", "b", "c", "d", "e"]));
    assert!(!p.can_undo());
    assert_eq!(p.undo_last(), None);
    p.graded("a", Grade::Fail, SECOND);
    p.graded("b", Grade::Pass, SECOND);
    assert_eq!(p.undo_last().as_deref(), Some("b"));
    assert_eq!(p.next(), Some("b"));
    // The failed card is taken out of its later place, and shown next.
    assert_eq!(p.undo_last().as_deref(), Some("a"));
    assert_eq!(p, Practice::new(ids(&["a", "b", "c", "d", "e"])));
}

#[test]
fn a_long_answer_counts_as_capped_time_and_undo_takes_exactly_that_back() {
    let mut p = Practice::new(ids(&["a", "b"]));
    p.graded("a", Grade::Pass, Duration::from_secs(3_600));
    assert_eq!(p.summary().time, capped(Duration::from_secs(3_600)));
    p.graded("b", Grade::Pass, SECOND);
    p.undo_last();
    assert_eq!(p.summary().time, capped(Duration::from_secs(3_600)));
}

#[test]
fn a_skipped_card_leaves_and_put_back_brings_it_next_once() {
    let mut p = Practice::new(ids(&["a", "b", "c"]));
    p.skip("a");
    assert_eq!(p.next(), Some("b"));
    assert_eq!(p.summary(), Summary::default());
    p.put_back("a");
    p.put_back("a");
    assert_eq!(p.queue, VecDeque::from(ids(&["a", "b", "c"])));
    // A card still queued is not moved.
    p.put_back("c");
    assert_eq!(p.queue, VecDeque::from(ids(&["a", "b", "c"])));
    // Skipping a card that is not there does nothing.
    p.skip("zz");
    assert_eq!(p.remaining(), 3);
}

#[test]
fn an_undo_after_a_skip_brings_the_card_back_once() {
    let mut p = Practice::new(ids(&["a", "b", "c", "d"]));
    p.graded("a", Grade::Fail, SECOND);
    // Suspended while back in the queue, then put back by its Undo.
    p.skip("a");
    p.put_back("a");
    p.undo_last();
    assert_eq!(p.queue.iter().filter(|id| *id == "a").count(), 1);
    assert_eq!(p.next(), Some("a"));
}

#[test]
fn repeated_ids_are_practised_once() {
    let p = Practice::new(ids(&["a", "b", "a"]));
    assert_eq!(p.queue, VecDeque::from(ids(&["a", "b"])));
}

proptest! {
    /// Answering the shown card any number of times, then undoing every
    /// answer, gives back the practice as it began; and a card is never
    /// queued twice.
    #[test]
    fn undoing_every_answer_is_the_identity(
        n in 1_usize..8,
        grades in proptest::collection::vec(any::<bool>(), 0..30),
    ) {
        let queue: Vec<String> = (0..n).map(|i| format!("c{i}")).collect();
        let start = Practice::new(queue);
        let mut p = start.clone();
        let mut answered = 0;
        for pass in grades {
            let Some(card) = p.next().map(str::to_owned) else { break };
            let grade = if pass { Grade::Pass } else { Grade::Fail };
            p.graded(&card, grade, SECOND);
            answered += 1;
            let mut seen = HashSet::new();
            prop_assert!(p.queue.iter().all(|id| seen.insert(id.clone())));
        }
        prop_assert_eq!(p.summary().answers, answered);
        while p.undo_last().is_some() {}
        prop_assert_eq!(p, start);
    }
}
