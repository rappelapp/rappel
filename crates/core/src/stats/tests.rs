use fsrs::MemoryState;
use proptest::prelude::*;

use super::*;
use crate::domain::{Card, CardState};
use crate::scheduling::{STEP_MS, Scheduler};
use crate::test_support::{DAY_MS, T0};

fn grid(start: i64, lengths: &[i64]) -> DayGrid {
    let mut bounds = vec![start];
    for l in lengths {
        bounds.push(bounds.last().unwrap() + l);
    }
    DayGrid::new(bounds).unwrap()
}

fn review(at: i64, passed: bool, duration_ms: i64) -> ActivityReview {
    ActivityReview {
        at,
        passed,
        duration_ms,
    }
}

#[test]
fn a_grid_needs_two_strictly_increasing_bounds() {
    for bad in [vec![], vec![5], vec![5, 5], vec![1, 3, 2]] {
        assert!(matches!(DayGrid::new(bad), Err(Error::InvalidDayGrid)));
    }
    assert_eq!(DayGrid::new(vec![1, 2]).unwrap().days(), 1);
}

#[test]
fn a_time_belongs_to_the_day_that_starts_at_or_before_it() {
    let g = grid(100, &[10, 10, 10]);
    let days: Vec<_> = [99, 100, 109, 110, 129, 130].map(|t| g.day_of(t)).into();
    assert_eq!(days, [None, Some(0), Some(0), Some(1), Some(2), None]);
}

#[test]
fn activity_counts_reviews_passes_and_time_per_day() {
    let g = grid(0, &[10, 10]);
    let days = activity_by_day(
        &g,
        [
            review(0, true, 5),
            review(9, false, 7),
            review(10, true, 11),
            review(20, true, 99),
            review(-1, true, 99),
        ],
    );
    assert_eq!(
        days,
        [
            DayActivity {
                reviews: 2,
                passes: 1,
                duration_ms: 12
            },
            DayActivity {
                reviews: 1,
                passes: 1,
                duration_ms: 11
            },
        ]
    );
    assert_eq!(pass_rate(&days), Some(2.0 / 3.0));
}

#[test]
fn no_reviews_have_no_pass_rate() {
    assert_eq!(pass_rate(&[DayActivity::default(); 3]), None);
    assert_eq!(pass_rate(&[]), None);
}

#[test]
fn days_of_23_and_25_hours_are_only_shorter_and_longer_buckets() {
    const H: i64 = 3_600_000;
    let g = grid(0, &[23 * H, 24 * H, 25 * H]);
    let days = activity_by_day(
        &g,
        [
            review(23 * H - 1, true, 1),
            review(23 * H, true, 1),
            review(47 * H, true, 1),
            review(72 * H - 1, true, 1),
            review(72 * H, true, 1),
        ],
    );
    assert_eq!(
        days.iter().map(|d| d.reviews).collect::<Vec<_>>(),
        [1, 1, 2]
    );
}

#[test]
fn the_forecast_puts_overdue_cards_on_day_zero() {
    let g = grid(1_000, &[100, 100, 100]);
    let (per_day, overdue) = due_forecast(&g, [0, 999, 1_000, 1_099, 1_100, 1_299, 1_300, 5_000]);
    assert_eq!((per_day, overdue), (vec![4, 1, 1], 2));
}

#[test]
fn retrievability_is_one_at_review_time_and_falls_with_time() {
    let at = |now| retrievability(5.0, 5.0, T0, now);
    assert!((at(T0) - 1.0).abs() < 1e-6);
    assert!(at(T0 + DAY_MS) > at(T0 + 10 * DAY_MS));
    // A clock before the last review is not a negative time.
    assert!((at(T0 - DAY_MS) - 1.0).abs() < 1e-6);
}

/// FSRS defines stability as the time for recall to fall to 90 %, so with
/// the scheduler's own interval for 0.9 retention the formula must give
/// 0.9: the decay used here is the one the scheduler uses.
#[test]
fn recall_after_the_scheduled_interval_is_the_desired_retention() {
    let scheduler = Scheduler::default();
    for stability in [0.5_f32, 3.0, 40.0, 900.0] {
        let days = scheduler.interval(stability);
        let r = recall_probability(
            MemoryState {
                stability,
                difficulty: 5.0,
            },
            days,
        );
        assert!((r - 0.9).abs() < 1e-3, "{stability}: {r}");
    }
}

#[test]
fn expected_retention_is_the_mean_over_scheduled_cards() {
    let card = |last_review_at| ScheduledCard {
        due_at: 0,
        stability: 5.0,
        difficulty: 5.0,
        last_review_at,
    };
    assert_eq!(expected_retention(T0, []), (0, None));
    let (n, mean) = expected_retention(T0 + 4 * DAY_MS, [card(T0), card(T0 + 4 * DAY_MS)]);
    let old = f64::from(retrievability(5.0, 5.0, T0, T0 + 4 * DAY_MS));
    assert_eq!(n, 2);
    assert!((mean.unwrap() - f64::midpoint(old, 1.0)).abs() < 1e-6);
}

// --- card information and the counted rule, on hand-made logs ---

fn entry(n: u32, at: i64, kind: EventKind) -> ReviewEvent {
    ReviewEvent {
        id: format!("e{n:03}"),
        card_id: "c".to_owned(),
        reviewed_at: at,
        kind,
        duration_ms: 1_000,
    }
}

fn card() -> Card {
    Card {
        id: "c".to_owned(),
        deck_id: "d".to_owned(),
        front_md: String::new(),
        back_md: String::new(),
        created_at: T0 - DAY_MS,
        updated_at: T0 - 1,
    }
}

fn pass() -> EventKind {
    EventKind::Review(Grade::Pass)
}

fn fail() -> EventKind {
    EventKind::Review(Grade::Fail)
}

#[test]
fn counted_reviews_skip_undone_ones_and_start_after_the_last_reset() {
    let log = [
        entry(1, T0, pass()),
        entry(2, T0 + 1, EventKind::Reset),
        entry(3, T0 + 2, pass()),
        entry(4, T0 + 3, fail()),
        entry(
            5,
            T0 + 4,
            EventKind::Undo {
                undoes: "e004".into(),
            },
        ),
        entry(6, T0 + 5, EventKind::Reset),
        entry(
            7,
            T0 + 6,
            EventKind::Undo {
                undoes: "e006".into(),
            },
        ),
        entry(8, T0 + 7, pass()),
    ];
    let ids: Vec<_> = counted_reviews(&log)
        .unwrap()
        .iter()
        .map(|e| e.id.clone())
        .collect();
    // The second reset is undone, so the first one starts the card's life.
    assert_eq!(ids, ["e003", "e008"]);
}

#[test]
fn activity_counts_reviews_before_a_reset_but_not_undone_ones() {
    let log = [
        entry(1, T0, pass()),
        entry(2, T0 + 1, EventKind::Reset),
        entry(3, T0 + 2, pass()),
        entry(4, T0 + 3, fail()),
        entry(
            5,
            T0 + 4,
            EventKind::Undo {
                undoes: "e004".into(),
            },
        ),
    ];
    let ids = |v: Vec<&ReviewEvent>| v.iter().map(|e| e.id.clone()).collect::<Vec<_>>();
    assert_eq!(ids(activity_reviews(&log).unwrap()), ["e001", "e003"]);
    assert_eq!(ids(counted_reviews(&log).unwrap()), ["e003"]);
}

#[test]
fn a_new_card_has_only_dates() {
    let info = card_info(&Scheduler::default(), &card(), &[], T0).unwrap();
    assert_eq!((info.created_at, info.updated_at), (T0 - DAY_MS, T0 - 1));
    assert_eq!((info.reviews, info.passes, info.lapses), (0, 0, 0));
    assert_eq!(info.pass_rate(), None);
    assert_eq!(
        (
            info.stability,
            info.difficulty,
            info.retrievability,
            info.next_due
        ),
        (None, None, None, None)
    );
    assert_eq!(info.history.len(), 0);
}

#[test]
fn card_info_counts_the_current_life_and_lists_the_whole_history() {
    let scheduler = Scheduler::default();
    let log = [
        entry(1, T0, pass()),
        entry(2, T0 + STEP_MS, pass()),
        entry(3, T0 + 5 * DAY_MS, EventKind::Reset),
        entry(4, T0 + 6 * DAY_MS, fail()),
        entry(5, T0 + 6 * DAY_MS + 1, pass()),
        entry(6, T0 + 6 * DAY_MS + 2, fail()),
        entry(
            7,
            T0 + 6 * DAY_MS + 3,
            EventKind::Undo {
                undoes: "e006".into(),
            },
        ),
    ];
    let now = T0 + 7 * DAY_MS;
    let info = card_info(&scheduler, &card(), &log, now).unwrap();
    assert_eq!((info.reviews, info.passes, info.lapses), (2, 1, 0));
    assert_eq!(info.pass_rate(), Some(0.5));
    let state = scheduler.rebuild_state("c", &log).unwrap();
    let CardState::Scheduled(s) = state else {
        panic!("scheduled")
    };
    assert_eq!(info.next_due, Some(s.due_at));
    assert_eq!(info.stability, Some(s.stability));
    assert_eq!(info.difficulty, Some(s.difficulty));
    assert_eq!(info.history.len(), 5);
    assert!(matches!(
        info.history[0],
        HistoryEntry::Review {
            grade: Grade::Pass,
            interval_ms: STEP_MS,
            ..
        }
    ));
    assert_eq!(
        info.history[2],
        HistoryEntry::Reset {
            at: T0 + 5 * DAY_MS
        }
    );
    // The undone fail is not in the history.
    assert!(matches!(
        info.history[4],
        HistoryEntry::Review {
            grade: Grade::Pass,
            ..
        }
    ));
}

#[test]
fn a_failed_review_card_counts_a_lapse() {
    let scheduler = Scheduler::default();
    let log = [
        entry(1, T0, pass()),
        entry(2, T0 + STEP_MS, pass()),
        entry(3, T0 + 30 * DAY_MS, fail()),
    ];
    let info = card_info(&scheduler, &card(), &log, T0 + 31 * DAY_MS).unwrap();
    assert_eq!((info.reviews, info.lapses), (3, 1));
}

#[test]
fn another_cards_entry_is_refused() {
    let mut other = entry(1, T0, pass());
    other.card_id = "x".into();
    assert!(matches!(
        card_info(&Scheduler::default(), &card(), &[other], T0),
        Err(Error::EventForOtherCard { .. })
    ));
}

#[test]
fn an_undo_without_a_target_is_refused() {
    let log = [entry(
        1,
        T0,
        EventKind::Undo {
            undoes: "e999".into(),
        },
    )];
    assert!(matches!(
        card_info(&Scheduler::default(), &card(), &log, T0),
        Err(Error::InvalidUndo { .. })
    ));
    assert!(matches!(
        counted_reviews(&log),
        Err(Error::InvalidUndo { .. })
    ));
}

proptest! {
    /// Bucketing agrees with a plain scan for any grid.
    #[test]
    fn day_of_agrees_with_a_scan(
        lengths in prop::collection::vec(1_i64..100, 1..20),
        t in -50_i64..2_000,
    ) {
        let g = grid(0, &lengths);
        let scan = (0..g.days()).find(|&i| g.0[i] <= t && t < g.0[i + 1]);
        prop_assert_eq!(g.day_of(t), scan);
    }

    /// Every review inside the grid lands in exactly one day.
    #[test]
    fn activity_loses_no_review(
        lengths in prop::collection::vec(1_i64..100, 1..20),
        times in prop::collection::vec(0_i64..2_000, 0..100),
    ) {
        let g = grid(0, &lengths);
        let days = activity_by_day(&g, times.iter().map(|&t| review(t, true, 2)));
        let inside = times.iter().filter(|&&t| t < g.end()).count();
        let total: u32 = days.iter().map(|d| d.reviews).sum();
        prop_assert_eq!(usize::try_from(total).unwrap(), inside);
    }
}
