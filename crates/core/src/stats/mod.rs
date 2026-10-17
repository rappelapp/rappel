//! Statistics over the review log, as pure functions (SPEC §4.7, ADR 0032).
//!
//! Every count uses the entries that survive (ADR 0028, 0032). Activity
//! (reviews, passes and time per day) counts every review no undo cancels,
//! whatever resets came later: the studying happened. [`activity_reviews`]
//! is that rule for one card's log; the store's SQL (`storage::stats`)
//! applies it and is tested against it. The memory side (schedule,
//! retention, forecast, and a card's reps and lapses) starts afresh at a
//! reset: [`counted_reviews`] is that rule.
//!
//! Core knows no time zones (ADR 0006, 0014). Days come from a
//! [`DayGrid`], the UTC timestamps at which consecutive study days begin,
//! which the app computes in local time. A day that is 23 or 25 hours long
//! is only a shorter or longer bucket, so daylight saving needs no code
//! here.

mod card;

pub(crate) use card::card_info;
pub use card::{CardInfo, HistoryEntry};

use fsrs::{FSRS6_DEFAULT_DECAY, MemoryState, current_retrievability};

use crate::domain::{EventKind, Grade, ReviewEvent};
use crate::error::{Error, Result};
use crate::scheduling::{DAY_MS, surviving};

/// The starts of consecutive study days, UTC milliseconds. `n + 1` strictly
/// increasing bounds describe `n` days: day `i` is `[bounds[i], bounds[i+1])`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DayGrid(Vec<i64>);

impl DayGrid {
    /// A grid from its bounds.
    ///
    /// # Errors
    /// [`Error::InvalidDayGrid`] unless there are at least two bounds and
    /// each is greater than the one before.
    pub fn new(bounds: Vec<i64>) -> Result<Self> {
        if bounds.len() < 2 || bounds.windows(2).any(|w| w[0] >= w[1]) {
            return Err(Error::InvalidDayGrid);
        }
        Ok(Self(bounds))
    }

    /// The number of days.
    #[must_use]
    pub fn days(&self) -> usize {
        self.0.len() - 1
    }

    /// When the first day begins.
    #[must_use]
    pub fn start(&self) -> i64 {
        self.0[0]
    }

    /// When the last day ends.
    #[must_use]
    pub fn end(&self) -> i64 {
        self.0[self.0.len() - 1]
    }

    /// The index of the day containing `t`, if the grid covers it.
    #[must_use]
    pub fn day_of(&self, t: i64) -> Option<usize> {
        (t >= self.start() && t < self.end()).then(|| self.0.partition_point(|&b| b <= t) - 1)
    }
}

/// What was done on one day.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct DayActivity {
    /// Counted reviews.
    pub reviews: u32,
    /// Of those, the passes.
    pub passes: u32,
    /// Time spent on them, milliseconds (each is capped when recorded).
    pub duration_ms: i64,
}

/// One review of the activity, as the statistics need it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ActivityReview {
    pub at: i64,
    pub passed: bool,
    pub duration_ms: i64,
}

/// The memory of a scheduled card, as the statistics need it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct ScheduledCard {
    pub due_at: i64,
    pub stability: f32,
    pub difficulty: f32,
    pub last_review_at: i64,
}

/// The statistics of a set of cards: everything, or one deck.
#[derive(Debug, Clone, PartialEq)]
pub struct Stats {
    /// Reviews, passes and time for each day of the past grid.
    pub days: Vec<DayActivity>,
    /// Passes over reviews in the past grid; `None` without reviews.
    pub pass_rate: Option<f64>,
    /// Cards due on each day of the ahead grid. Day 0 also takes everything
    /// due earlier (overdue cards); those cards are counted again in
    /// `overdue`.
    pub forecast: Vec<u32>,
    /// Cards due before the ahead grid begins.
    pub overdue: u32,
    /// Cards that have a schedule (reviewed, and not reset since).
    pub scheduled_cards: u32,
    /// The mean probability of recall now over those cards (FSRS
    /// retrievability); `None` without any.
    pub expected_retention: Option<f64>,
}

impl Stats {
    /// The statistics of `reviews` (the surviving ones; those outside `past`
    /// are ignored) and `cards` (every live scheduled card) at `now`.
    pub(crate) fn compute(
        past: &DayGrid,
        ahead: &DayGrid,
        now: i64,
        reviews: impl IntoIterator<Item = ActivityReview>,
        cards: impl IntoIterator<Item = ScheduledCard> + Clone,
    ) -> Self {
        let days = activity_by_day(past, reviews);
        let (forecast, overdue) = due_forecast(ahead, cards.clone().into_iter().map(|c| c.due_at));
        let (scheduled_cards, expected_retention) = expected_retention(now, cards);
        Self {
            pass_rate: pass_rate(&days),
            days,
            forecast,
            overdue,
            scheduled_cards,
            expected_retention,
        }
    }
}

/// Reviews, passes and time per day of `grid`.
pub(crate) fn activity_by_day(
    grid: &DayGrid,
    reviews: impl IntoIterator<Item = ActivityReview>,
) -> Vec<DayActivity> {
    let mut days = vec![DayActivity::default(); grid.days()];
    for r in reviews {
        if let Some(day) = grid.day_of(r.at).map(|i| &mut days[i]) {
            day.reviews = day.reviews.saturating_add(1);
            day.passes = day.passes.saturating_add(u32::from(r.passed));
            day.duration_ms = day.duration_ms.saturating_add(r.duration_ms);
        }
    }
    days
}

/// Passes over reviews in `days`; `None` if there are no reviews.
pub(crate) fn pass_rate(days: &[DayActivity]) -> Option<f64> {
    let (reviews, passes) = days.iter().fold((0_u64, 0_u64), |(r, p), d| {
        (r + u64::from(d.reviews), p + u64::from(d.passes))
    });
    #[expect(
        clippy::cast_precision_loss,
        reason = "counts of reviews stay far below 2^53"
    )]
    (reviews > 0).then(|| passes as f64 / reviews as f64)
}

/// How many of `due` (due times) fall on each day of `grid`, and how many
/// are before it. Day 0 includes those; times after the grid are left out.
pub(crate) fn due_forecast(grid: &DayGrid, due: impl IntoIterator<Item = i64>) -> (Vec<u32>, u32) {
    let mut per_day = vec![0_u32; grid.days()];
    let mut overdue = 0_u32;
    for t in due {
        let day = if t < grid.start() {
            overdue = overdue.saturating_add(1);
            Some(0)
        } else {
            grid.day_of(t)
        };
        if let Some(i) = day {
            per_day[i] = per_day[i].saturating_add(1);
        }
    }
    (per_day, overdue)
}

/// The probability of recalling a card with `memory` after `elapsed_days`,
/// by the `fsrs` crate's own formula with the decay of the default FSRS-6
/// parameters, which the scheduler uses.
pub(crate) fn recall_probability(memory: MemoryState, elapsed_days: f32) -> f32 {
    current_retrievability(memory, elapsed_days, FSRS6_DEFAULT_DECAY)
}

/// The probability of recall of a card at `now`. Elapsed time is a
/// fraction of days (to the second), not the scheduler's rounded whole
/// days: this is a value for the present moment.
pub(crate) fn retrievability(
    stability: f32,
    difficulty: f32,
    last_review_at: i64,
    now: i64,
) -> f32 {
    let elapsed_ms = now.saturating_sub(last_review_at).max(0);
    #[expect(
        clippy::cast_precision_loss,
        reason = "a span of days in f32 is as precise as FSRS itself works"
    )]
    let days = (elapsed_ms / 1000) as f32 / (DAY_MS / 1000) as f32;
    recall_probability(
        MemoryState {
            stability,
            difficulty,
        },
        days,
    )
}

/// The number of cards and the mean probability of recall at `now`;
/// `None` for no cards.
pub(crate) fn expected_retention(
    now: i64,
    cards: impl IntoIterator<Item = ScheduledCard>,
) -> (u32, Option<f64>) {
    let (count, sum) = cards.into_iter().fold((0_u32, 0.0_f64), |(n, sum), c| {
        let r = retrievability(c.stability, c.difficulty, c.last_review_at, now);
        (n.saturating_add(1), sum + f64::from(r))
    });
    (count, (count > 0).then(|| sum / f64::from(count)))
}

/// The reviews of one card's log that its memory counts: not undone, after
/// the last surviving reset, oldest first (ADR 0028).
///
/// # Errors
/// [`Error::InvalidUndo`], as [`surviving`].
pub(crate) fn counted_reviews(log: &[ReviewEvent]) -> Result<Vec<&ReviewEvent>> {
    let mut kept = surviving(log)?;
    let after_reset = kept
        .iter()
        .rposition(|e| matches!(e.kind, EventKind::Reset))
        .map_or(0, |i| i + 1);
    kept.drain(..after_reset);
    Ok(kept)
}

/// The reviews of one card's log that its activity counts: not undone,
/// whatever resets came later, oldest first (ADR 0032).
///
/// # Errors
/// [`Error::InvalidUndo`], as [`surviving`].
///
/// The store counts these in SQL; this is the rule the tests check it against.
#[cfg(test)]
pub(crate) fn activity_reviews(log: &[ReviewEvent]) -> Result<Vec<&ReviewEvent>> {
    let mut kept = surviving(log)?;
    kept.retain(|e| matches!(e.kind, EventKind::Review(_)));
    Ok(kept)
}

/// The review `e` as the statistics need it; `None` for a reset or
/// an undo.
pub(crate) fn as_activity(e: &ReviewEvent) -> Option<ActivityReview> {
    Some(ActivityReview {
        at: e.reviewed_at,
        passed: e.grade()? == Grade::Pass,
        duration_ms: e.duration_ms,
    })
}

#[cfg(test)]
mod tests;
