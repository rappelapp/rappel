//! What the card information screen shows (issue #39, ADR 0032).

use super::{as_activity, counted_reviews, retrievability};
use crate::domain::{Card, CardState, EventKind, Grade, ReviewEvent};
use crate::error::{Error, Result};
use crate::scheduling::{Scheduler, surviving};

/// One surviving entry of a card's history.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HistoryEntry {
    /// An answer.
    Review {
        /// When, UTC milliseconds.
        at: i64,
        /// The answer.
        grade: Grade,
        /// Time spent, milliseconds.
        duration_ms: i64,
        /// How far ahead the answer scheduled the card (the 10-minute step
        /// included), milliseconds.
        interval_ms: i64,
    },
    /// The card's progress was reset here; the reviews after it start afresh.
    Reset {
        /// When, UTC milliseconds.
        at: i64,
    },
}

/// The properties of one card. Counts and memory cover the card's current
/// life, since its last surviving reset.
#[derive(Debug, Clone, PartialEq)]
pub struct CardInfo {
    /// When the card was created, UTC milliseconds.
    pub created_at: i64,
    /// When it was last edited (or created), UTC milliseconds.
    pub updated_at: i64,
    /// Counted reviews.
    pub reviews: u32,
    /// Of those, the passes.
    pub passes: u32,
    /// Times it was failed while in review.
    pub lapses: u32,
    /// FSRS stability in days; `None` for a new card.
    pub stability: Option<f32>,
    /// FSRS difficulty, 1 to 10; `None` for a new card.
    pub difficulty: Option<f32>,
    /// The probability of recall now; `None` for a new card.
    pub retrievability: Option<f32>,
    /// When the card is next due, UTC milliseconds; `None` for a new card.
    pub next_due: Option<i64>,
    /// The surviving reviews and resets, oldest first. Undone entries are
    /// left out.
    pub history: Vec<HistoryEntry>,
    /// The card is suspended: excluded from study (ADR 0035).
    pub suspended: bool,
}

impl CardInfo {
    /// Passes over counted reviews; `None` without reviews.
    #[must_use]
    pub fn pass_rate(&self) -> Option<f64> {
        (self.reviews > 0).then(|| f64::from(self.passes) / f64::from(self.reviews))
    }
}

/// The information of `card` from its `log`, at `now`. The schedule is
/// replayed from the log, as the cache is, so the two always agree.
///
/// # Errors
/// [`Error::EventForOtherCard`], [`Error::InvalidUndo`], and the scheduler's
/// errors.
pub(crate) fn card_info(
    scheduler: &Scheduler,
    card: &Card,
    log: &[ReviewEvent],
    now: i64,
) -> Result<CardInfo> {
    if let Some(e) = log.iter().find(|e| e.card_id != card.id) {
        return Err(Error::EventForOtherCard {
            event_id: e.id.clone(),
            card_id: card.id.clone(),
        });
    }
    let mut state = CardState::New;
    let mut history = Vec::new();
    for e in surviving(log)? {
        match e.kind {
            EventKind::Review(grade) => {
                let next = scheduler.schedule(&card.id, &state, grade, e.reviewed_at)?;
                // `schedule` always returns a scheduled card; the replay is
                // `Scheduler::rebuild_state`'s fold, kept here to see each step.
                if let CardState::Scheduled(s) = next {
                    history.push(HistoryEntry::Review {
                        at: e.reviewed_at,
                        grade,
                        duration_ms: e.duration_ms,
                        interval_ms: s.due_at.saturating_sub(e.reviewed_at),
                    });
                }
                state = next;
            }
            EventKind::Reset => {
                history.push(HistoryEntry::Reset { at: e.reviewed_at });
                state = CardState::New;
            }
            // `surviving` drops every undo.
            EventKind::Undo { .. } => {}
        }
    }
    let counted: Vec<_> = counted_reviews(log)?
        .into_iter()
        .filter_map(as_activity)
        .collect();
    let (reviews, passes) = (
        u32::try_from(counted.len()).unwrap_or(u32::MAX),
        u32::try_from(counted.iter().filter(|r| r.passed).count()).unwrap_or(u32::MAX),
    );
    let schedule = match state {
        CardState::New => None,
        CardState::Scheduled(s) => Some(s),
    };
    Ok(CardInfo {
        created_at: card.created_at,
        updated_at: card.updated_at,
        reviews,
        passes,
        lapses: schedule.map_or(0, |s| s.lapses),
        stability: schedule.map(|s| s.stability),
        difficulty: schedule.map(|s| s.difficulty),
        retrievability: schedule
            .map(|s| retrievability(s.stability, s.difficulty, s.last_review_at, now)),
        next_due: schedule.map(|s| s.due_at),
        suspended: card.suspended,
        history,
    })
}
