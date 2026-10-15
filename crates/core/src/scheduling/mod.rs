//! FSRS scheduling: the next state of a card after a review, and rebuilding
//! a card's state from its review log.
//!
//! See ADR 0006. In short:
//! - Fail is FSRS rating 1 (Again), Pass is rating 3 (Good).
//! - A new card, and any card that is failed, gets one 10-minute step. A
//!   Pass from the step (or from review) schedules it in whole days with
//!   FSRS, fuzzed deterministically.
//! - Every review updates the FSRS memory state, including same-day ones
//!   (FSRS-6 has its own short-term formula for elapsed days = 0).
//! - Elapsed days are whole 24-hour periods since the last review, rounded
//!   to the nearest: no time zone is involved, so a rebuild needs nothing
//!   but the stored UTC timestamps.
//!
//! [`Scheduler::rebuild_state`] is a fold of [`Scheduler::schedule`] over
//! the events in `(reviewed_at, id)` order. For the cache and a rebuild to
//! agree, the log must already be in that order, which `schedule` enforces
//! for times: it refuses a review earlier than the card's last one. Code
//! that records reviews must store `reviewed_at = max(now, last_review_at)`
//! (a clock can go backwards) and use a `UUIDv7` generator that is
//! monotonic within a millisecond and across restarts (ADR 0008:
//! `storage::record_review` observes the log's largest id first).

pub(crate) mod fuzz;
pub mod session;

use fsrs::{FSRS, MemoryState};

use crate::domain::{CardState, Grade, Phase, ReviewEvent, Schedule};
use crate::error::{Error, Result};
use fuzz::{floored_fuzzed_days, fuzz_factor};

/// One day in milliseconds.
pub const DAY_MS: i64 = 86_400_000;

/// Length of the short-term step for new and failed cards, milliseconds.
pub const STEP_MS: i64 = 10 * 60 * 1000;

/// The desired retention used until the user changes it (SPEC §3).
pub const DEFAULT_DESIRED_RETENTION: f32 = 0.90;

/// The range of desired retention the user may choose (SPEC §3).
pub const DESIRED_RETENTION_RANGE: std::ops::RangeInclusive<f32> = 0.80..=0.97;

/// FSRS rating for Pass ("Good"). Fail is rating 1 ("Again"); `schedule`
/// picks the matching field of `NextStates` rather than passing ratings.
const RATING_GOOD: u32 = 3;

/// Schedules reviews with FSRS-6 and its default parameters.
#[derive(Debug, Clone)]
pub struct Scheduler {
    fsrs: FSRS,
    desired_retention: f32,
}

impl Default for Scheduler {
    /// Default FSRS parameters, desired retention 0.90.
    fn default() -> Self {
        Self {
            fsrs: FSRS::default(),
            desired_retention: DEFAULT_DESIRED_RETENTION,
        }
    }
}

impl Scheduler {
    /// A scheduler with default FSRS parameters and the given desired
    /// retention (probability of recall when a card comes due).
    ///
    /// # Errors
    /// [`Error::InvalidRetention`] if `desired_retention` is outside
    /// [`DESIRED_RETENTION_RANGE`].
    pub fn new(desired_retention: f32) -> Result<Self> {
        if !DESIRED_RETENTION_RANGE.contains(&desired_retention) {
            return Err(Error::InvalidRetention(desired_retention));
        }
        Ok(Self {
            desired_retention,
            ..Self::default()
        })
    }

    /// The desired retention this scheduler targets.
    #[must_use]
    pub fn desired_retention(&self) -> f32 {
        self.desired_retention
    }

    /// The state of card `card_id` after it was answered with `grade` at
    /// `now` (UTC milliseconds), starting from `state`.
    ///
    /// | from       | Fail                          | Pass                  |
    /// |------------|-------------------------------|-----------------------|
    /// | new        | learning, +10 min             | learning, +10 min     |
    /// | learning   | learning, +10 min             | review, FSRS interval |
    /// | review     | relearning, +10 min, lapse +1 | review, FSRS interval, at least elapsed + 1 days |
    /// | relearning | relearning, +10 min           | review, FSRS interval |
    ///
    /// The fuzz depends on `card_id` and the number of earlier reviews, so
    /// the result is a pure function of the arguments.
    ///
    /// # Errors
    /// [`Error::ReviewBeforeLastReview`] if `now` is earlier than the
    /// card's last review: replaying the log sorts by time, so such a
    /// review would be applied in a different order on rebuild.
    /// [`Error::Fsrs`] if FSRS produces a non-finite memory state, which
    /// only happens if `state` holds non-finite values.
    pub fn schedule(
        &self,
        card_id: &str,
        state: &CardState,
        grade: Grade,
        now: i64,
    ) -> Result<CardState> {
        if let CardState::Scheduled(s) = state
            && now < s.last_review_at
        {
            return Err(Error::ReviewBeforeLastReview {
                reviewed_at: now,
                last_review_at: s.last_review_at,
            });
        }
        let (memory, elapsed, phase, reps, lapses) = match *state {
            CardState::New => (None, 0, None, 0, 0),
            CardState::Scheduled(s) => (
                Some(MemoryState {
                    stability: s.stability,
                    difficulty: s.difficulty,
                }),
                elapsed_days(s.last_review_at, now),
                Some(s.phase),
                s.reps,
                s.lapses,
            ),
        };
        let next = self
            .fsrs
            .next_states(memory, self.desired_retention, elapsed)?;
        let memory = match grade {
            Grade::Fail => next.again.memory,
            Grade::Pass => next.good.memory,
        };
        let step = now.saturating_add(STEP_MS);
        let (phase, due_at, lapses) = match (phase, grade) {
            (None, _) | (Some(Phase::Learning), Grade::Fail) => (Phase::Learning, step, lapses),
            (Some(Phase::Review), Grade::Fail) => {
                (Phase::Relearning, step, lapses.saturating_add(1))
            }
            (Some(Phase::Relearning), Grade::Fail) => (Phase::Relearning, step, lapses),
            (Some(from), Grade::Pass) => {
                let interval = self.interval(memory.stability);
                let floor = pass_floor(from, elapsed);
                let days =
                    floored_fuzzed_days(f64::from(interval), floor, fuzz_factor(card_id, reps));
                let due_at = now.saturating_add(i64::from(days).saturating_mul(DAY_MS));
                (Phase::Review, due_at, lapses)
            }
        };
        Ok(CardState::Scheduled(Schedule {
            phase,
            stability: memory.stability,
            difficulty: memory.difficulty,
            due_at,
            last_review_at: now,
            reps: reps.saturating_add(1),
            lapses,
        }))
    }

    /// The state of card `card_id` after replaying `events`, its review
    /// log. Events are applied in `(reviewed_at, id)` order whatever their
    /// order in the slice. Ids are `UUIDv7`; with a generator that is
    /// monotonic within a millisecond, equal timestamps keep the order they
    /// were recorded in.
    ///
    /// The result is exactly that of calling [`Self::schedule`] once per
    /// event in that order, starting from [`CardState::New`]. A log built
    /// by successful `schedule` calls is already in that order, so its
    /// rebuild equals the state those calls produced.
    ///
    /// # Errors
    /// [`Error::EventForOtherCard`] if an event belongs to another card;
    /// errors of [`Self::schedule`].
    pub fn rebuild_state(&self, card_id: &str, events: &[ReviewEvent]) -> Result<CardState> {
        if let Some(e) = events.iter().find(|e| e.card_id != card_id) {
            return Err(Error::EventForOtherCard {
                event_id: e.id.clone(),
                card_id: card_id.to_owned(),
            });
        }
        let mut ordered: Vec<&ReviewEvent> = events.iter().collect();
        ordered.sort_by(|a, b| (a.reviewed_at, &a.id).cmp(&(b.reviewed_at, &b.id)));
        ordered.into_iter().try_fold(CardState::New, |state, e| {
            self.schedule(card_id, &state, e.grade, e.reviewed_at)
        })
    }

    /// FSRS's unrounded, unfuzzed interval in days for `stability`.
    pub(crate) fn interval(&self, stability: f32) -> f32 {
        // The rating argument is only used when stability is `None`.
        self.fsrs
            .next_interval(Some(stability), self.desired_retention, RATING_GOOD)
    }
}

/// The fewest days a Pass from phase `from` may be scheduled to, given the
/// whole days `elapsed` since the last review (ADR 0009). A review card
/// that was recalled after `elapsed` days gets at least `elapsed + 1`
/// (Anki's floor), so a late Pass never shortens its interval below the
/// gap it just survived. A Pass from the 10-minute step has no floor
/// beyond one day.
fn pass_floor(from: Phase, elapsed: u32) -> u32 {
    match from {
        Phase::Review => elapsed.saturating_add(1),
        Phase::Learning | Phase::Relearning => 1,
    }
}

/// Whole days from `last` to `now`, rounded to the nearest (12 h rounds
/// up). Negative spans count as 0 days (`schedule` refuses them anyway).
fn elapsed_days(last: i64, now: i64) -> u32 {
    let elapsed = now.saturating_sub(last).max(0);
    let days = elapsed / DAY_MS + i64::from(elapsed % DAY_MS >= DAY_MS / 2);
    u32::try_from(days).unwrap_or(u32::MAX)
}

#[cfg(test)]
mod tests;
