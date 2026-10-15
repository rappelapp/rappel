//! Domain types shared by storage and scheduling.

use std::fmt;
use std::str::FromStr;

/// The answer to a review. Cards has exactly two grades (SPEC §6).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Grade {
    /// The answer was not recalled.
    Fail,
    /// The answer was recalled.
    Pass,
}

impl Grade {
    /// The value stored in `review_events.grade`.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Fail => "fail",
            Self::Pass => "pass",
        }
    }
}

impl fmt::Display for Grade {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// A string that is not a [`Grade`].
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("not a grade: {0:?}")]
pub struct ParseGradeError(pub String);

impl FromStr for Grade {
    type Err = ParseGradeError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "fail" => Ok(Self::Fail),
            "pass" => Ok(Self::Pass),
            other => Err(ParseGradeError(other.to_owned())),
        }
    }
}

/// One row of `review_events`: a card was answered at a point in time.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReviewEvent {
    /// Event id (`UUIDv7` string).
    pub id: String,
    /// The reviewed card's id.
    pub card_id: String,
    /// When the answer was given, UTC milliseconds.
    pub reviewed_at: i64,
    /// The answer.
    pub grade: Grade,
    /// Time spent on the card, milliseconds.
    pub duration_ms: i64,
}

/// Where a card that has been reviewed at least once stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Phase {
    /// Reviewed but not yet passed after the first showing: in the
    /// 10-minute step.
    Learning,
    /// Passed: scheduled in whole days by FSRS.
    Review,
    /// Failed while in review: in the 10-minute step again.
    Relearning,
}

impl Phase {
    /// The value stored in `card_schedule.phase`.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Learning => "learning",
            Self::Review => "review",
            Self::Relearning => "relearning",
        }
    }
}

/// A string that is not a [`Phase`].
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("not a phase: {0:?}")]
pub struct ParsePhaseError(pub String);

impl FromStr for Phase {
    type Err = ParsePhaseError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "learning" => Ok(Self::Learning),
            "review" => Ok(Self::Review),
            "relearning" => Ok(Self::Relearning),
            other => Err(ParsePhaseError(other.to_owned())),
        }
    }
}

/// The scheduling state of one card, as cached in `card_schedule`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum CardState {
    /// Never reviewed (no `card_schedule` row).
    New,
    /// Reviewed at least once.
    Scheduled(Schedule),
}

/// Scheduling state of a card that has been reviewed at least once.
///
/// `stability` and `difficulty` are FSRS's `f32` values. SQLite stores them
/// as `REAL` (`f64`), which holds every `f32` exactly, so the cache and a
/// rebuild compare equal bit for bit.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Schedule {
    /// Learning, review or relearning.
    pub phase: Phase,
    /// FSRS stability in days: the interval after which recall probability
    /// drops to 90 %.
    pub stability: f32,
    /// FSRS difficulty, 1 (easy) to 10 (hard).
    pub difficulty: f32,
    /// When the card is next due, UTC milliseconds.
    pub due_at: i64,
    /// When the card was last reviewed, UTC milliseconds.
    pub last_review_at: i64,
    /// Number of reviews so far.
    pub reps: u32,
    /// Number of times the card was failed while in review.
    pub lapses: u32,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn grade_round_trips_through_its_db_string() {
        for g in [Grade::Fail, Grade::Pass] {
            assert_eq!(g.as_str().parse::<Grade>(), Ok(g));
            assert_eq!(g.to_string(), g.as_str());
        }
        assert!("Pass".parse::<Grade>().is_err());
        assert!("again".parse::<Grade>().is_err());
    }

    #[test]
    fn phase_round_trips_through_its_db_string() {
        for p in [Phase::Learning, Phase::Review, Phase::Relearning] {
            assert_eq!(p.as_str().parse::<Phase>(), Ok(p));
        }
        assert!("new".parse::<Phase>().is_err());
    }
}
