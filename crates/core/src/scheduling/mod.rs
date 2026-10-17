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
//! `Scheduler::rebuild_state` is a fold of `Scheduler::schedule` over
//! the events in `(reviewed_at, id)` order. For the cache and a rebuild to
//! agree, the log must already be in that order, which `schedule` enforces
//! for times: it refuses a review earlier than the card's last one. Code
//! that records reviews must store `reviewed_at = max(now, last_review_at)`
//! (a clock can go backwards) and use a `UUIDv7` generator that is
//! monotonic within a millisecond and across restarts (ADR 0008:
//! `storage::record_review` observes the log's largest id first).

pub(crate) mod fuzz;
pub(crate) mod session;

pub use session::{Due, Session, StudyInput, Summary, capped};

use std::collections::HashSet;

use fsrs::{FSRS, MemoryState};

use crate::domain::{
    CardState, DEFAULT_DESIRED_RETENTION, EventKind, Grade, Phase, ReviewEvent, Schedule, Settings,
};
use crate::error::{Error, Result};
use fuzz::{floored_fuzzed_days, fuzz_factor};

/// One day in milliseconds.
pub const DAY_MS: i64 = 86_400_000;

/// Length of the short-term step for new and failed cards, milliseconds.
pub const STEP_MS: i64 = 10 * 60 * 1000;

/// FSRS rating for Pass ("Good"). Fail is rating 1 ("Again"); `schedule`
/// picks the matching field of `NextStates` rather than passing ratings.
const RATING_GOOD: u32 = 3;

/// Schedules reviews with FSRS-6 and its default parameters.
#[derive(Debug, Clone)]
pub(crate) struct Scheduler {
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
    /// [`Error::InvalidSetting`] if `desired_retention` is outside
    /// [`DESIRED_RETENTION_RANGE`](crate::domain::DESIRED_RETENTION_RANGE).
    pub fn new(desired_retention: f32) -> Result<Self> {
        Ok(Self {
            desired_retention: Settings::check_retention(desired_retention)?,
            ..Self::default()
        })
    }

    /// The desired retention this scheduler targets.
    #[cfg(test)]
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

    /// The state of card `card_id` after replaying `events`, its log
    /// (ADR 0028):
    /// 1. every undo, and every entry an undo cancels, is dropped;
    /// 2. the rest is ordered by `(reviewed_at, id)`, whatever its order in
    ///    the slice (ids are `UUIDv7`; with a generator that is monotonic
    ///    within a millisecond, equal timestamps keep the order they were
    ///    recorded in);
    /// 3. starting from [`CardState::New`], a review is one
    ///    [`Self::schedule`] call and a reset makes the card new again.
    ///
    /// So `reps`, `lapses` and the fuzz seed count only the surviving
    /// reviews since the last surviving reset (#11). A log built by
    /// successful `schedule` calls is already in that order, so its rebuild
    /// equals the state those calls produced.
    ///
    /// # Errors
    /// [`Error::EventForOtherCard`] if an entry belongs to another card;
    /// [`Error::InvalidUndo`] if an undo names no review or reset in
    /// `events`, or an entry is undone twice; errors of [`Self::schedule`].
    pub fn rebuild_state(&self, card_id: &str, events: &[ReviewEvent]) -> Result<CardState> {
        if let Some(e) = events.iter().find(|e| e.card_id != card_id) {
            return Err(Error::EventForOtherCard {
                event_id: e.id.clone(),
                card_id: card_id.to_owned(),
            });
        }
        surviving(events)?
            .into_iter()
            .try_fold(CardState::New, |state, e| match e.kind {
                EventKind::Review(grade) => self.schedule(card_id, &state, grade, e.reviewed_at),
                EventKind::Reset => Ok(CardState::New),
                // `surviving` drops every undo.
                EventKind::Undo { .. } => Ok(state),
            })
    }

    /// FSRS's unrounded, unfuzzed interval in days for `stability`.
    pub(crate) fn interval(&self, stability: f32) -> f32 {
        // The rating argument is only used when stability is `None`.
        self.fsrs
            .next_interval(Some(stability), self.desired_retention, RATING_GOOD)
    }
}

/// The reviews and resets of one card's log that no undo cancels, in
/// replay order `(reviewed_at, id)`; undos themselves are left out.
///
/// # Errors
/// [`Error::InvalidUndo`] if an undo names no review or reset in `events`,
/// or two undos name the same entry.
pub(crate) fn surviving(events: &[ReviewEvent]) -> Result<Vec<&ReviewEvent>> {
    // Linear: a file may hold a card with very many undos (ADR 0025).
    let targets: HashSet<&str> = events
        .iter()
        .filter(|e| !matches!(e.kind, EventKind::Undo { .. }))
        .map(|e| e.id.as_str())
        .collect();
    let mut undone: HashSet<&str> = HashSet::new();
    for e in events {
        if let EventKind::Undo { undoes } = &e.kind
            && (!targets.contains(undoes.as_str()) || !undone.insert(undoes))
        {
            return Err(Error::InvalidUndo {
                event_id: e.id.clone(),
            });
        }
    }
    let mut kept: Vec<&ReviewEvent> = events
        .iter()
        .filter(|e| !matches!(e.kind, EventKind::Undo { .. }) && !undone.contains(e.id.as_str()))
        .collect();
    kept.sort_by(|a, b| (a.reviewed_at, &a.id).cmp(&(b.reviewed_at, &b.id)));
    Ok(kept)
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
