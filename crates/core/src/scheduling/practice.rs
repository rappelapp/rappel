//! Practice (ADR 0070): going over a deck's or a tag's cards outside the
//! schedule, with the same Fail and Pass, **writing nothing**. Pure: no
//! database and no clock.
//!
//! FSRS learns from spaced recalls. A recall seconds or minutes after the
//! last review (elapsed time near 0) carries almost no information about
//! memory, and feeding it to the scheduler would distort stability and
//! difficulty; the review log is the source of truth and append-only
//! (ADR 0028). So a practice answer is kept only here, in memory, for the
//! summary, and nothing in this module can reach a [`Store`]: [`Practice`]
//! holds plain card ids. The store calls practice needs,
//! [`Store::practice_queue`] and `has_practisable`, read in one read
//! transaction with `PRAGMA query_only` on, so a write there fails.
//!
//! Order: [`practice_order`], lowest retrievability first, then the new
//! cards, oldest first, at most [`PRACTICE_LIMIT`]. A Fail brings the card
//! back [`RELEARN_GAP`] cards later, a short relearning in memory.
//!
//! [`Store`]: crate::storage::Store
//! [`Store::practice_queue`]: crate::storage::Store::practice_queue

use std::cmp::Ordering;
use std::collections::{HashSet, VecDeque};
use std::time::Duration;

use super::session::{Summary, capped};
use crate::domain::Grade;

/// The most cards one practice offers: the least well remembered ones. A
/// practice is a short refresher, not a second pass over a whole deck of
/// thousands; another practice starts from the top again.
pub(crate) const PRACTICE_LIMIT: usize = 50;

/// How many other cards come before a failed card comes back in the same
/// practice (fewer when fewer are left).
pub const RELEARN_GAP: usize = 3;

/// A card that may be practised, for [`practice_order`]: a live, not
/// suspended card of the scope, in a deck that is not archived.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct PracticeCandidate {
    /// The card's id.
    pub card_id: String,
    /// Its creation order (`cards.seq`): "oldest first" among new cards.
    pub seq: i64,
    /// Its probability of recall now (`stats::retrievability`), `None` for
    /// a new card, which has no memory state.
    pub retrievability: Option<f32>,
}

/// The order of a practice: the reviewed cards by retrievability, lowest
/// first (ties by card id), then the new cards by `seq` (ties by card id),
/// cut to [`PRACTICE_LIMIT`]. Deterministic for any input order.
#[must_use]
pub(crate) fn practice_order(mut cards: Vec<PracticeCandidate>) -> Vec<String> {
    cards.sort_by(|a, b| match (a.retrievability, b.retrievability) {
        (Some(ra), Some(rb)) => ra.total_cmp(&rb).then_with(|| a.card_id.cmp(&b.card_id)),
        (Some(_), None) => Ordering::Less,
        (None, Some(_)) => Ordering::Greater,
        (None, None) => (a.seq, &a.card_id).cmp(&(b.seq, &b.card_id)),
    });
    cards
        .into_iter()
        .take(PRACTICE_LIMIT)
        .map(|c| c.card_id)
        .collect()
}

/// What one [`Practice::graded`] changed, so [`Practice::undo_last`] can
/// put it back.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Answer {
    card_id: String,
    grade: Grade,
    /// Whether the card was answered for the first time in this practice.
    newly_seen: bool,
    /// The capped duration added to the summary.
    time: Duration,
}

/// One practice: a queue of card ids and the answers given, in memory
/// only. Nothing here is written anywhere; dropping it leaves no trace.
///
/// A card is in the queue at most once. The card shown is the queue's
/// first until it is answered or skipped.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Practice {
    queue: VecDeque<String>,
    seen: HashSet<String>,
    summary: Summary,
    /// The answers that can be undone, oldest first.
    answers: Vec<Answer>,
}

impl Practice {
    /// A practice over `queue` (as [`Store::practice_queue`] gives it), in
    /// that order. Repeated ids are kept once, at their first place.
    ///
    /// [`Store::practice_queue`]: crate::storage::Store::practice_queue
    #[must_use]
    pub fn new(queue: Vec<String>) -> Self {
        let mut held = HashSet::new();
        let queue = queue
            .into_iter()
            .filter(|id| held.insert(id.clone()))
            .collect();
        Self {
            queue,
            seen: HashSet::new(),
            summary: Summary::default(),
            answers: Vec::new(),
        }
    }

    /// The card to show, or `None` when the practice is over.
    #[must_use]
    pub fn next(&self) -> Option<&str> {
        self.queue.front().map(String::as_str)
    }

    /// Cards still waiting, the one on screen included.
    #[must_use]
    pub fn remaining(&self) -> usize {
        self.queue.len()
    }

    /// Records, in memory, that `card_id` was answered with `grade` after
    /// `duration`. A Pass takes it out of the practice; a Fail brings it
    /// back after [`RELEARN_GAP`] other cards (at the end if fewer are
    /// left, so at once when it is the last). The answer can be taken back
    /// with [`Self::undo_last`].
    pub fn graded(&mut self, card_id: &str, grade: Grade, duration: Duration) {
        self.queue.retain(|id| id != card_id);
        if grade == Grade::Fail {
            let at = RELEARN_GAP.min(self.queue.len());
            self.queue.insert(at, card_id.to_owned());
        }
        let newly_seen = self.seen.insert(card_id.to_owned());
        if newly_seen {
            self.summary.cards += 1;
        }
        self.summary.answers += 1;
        if grade == Grade::Pass {
            self.summary.passes += 1;
        }
        let time = capped(duration);
        self.summary.time += time;
        self.answers.push(Answer {
            card_id: card_id.to_owned(),
            grade,
            newly_seen,
            time,
        });
    }

    /// Whether there is an answer to take back.
    #[must_use]
    pub fn can_undo(&self) -> bool {
        !self.answers.is_empty()
    }

    /// Takes back the newest answer: the summary loses it and the card is
    /// shown next (taken from where a Fail put it). Only memory changes:
    /// there is no log entry to undo. Returns the card's id; `None` when
    /// there is nothing to undo.
    pub fn undo_last(&mut self) -> Option<String> {
        let answer = self.answers.pop()?;
        let id = answer.card_id;
        self.queue.retain(|c| *c != id);
        self.queue.push_front(id.clone());
        if answer.newly_seen {
            self.seen.remove(&id);
            self.summary.cards -= 1;
        }
        self.summary.answers -= 1;
        if answer.grade == Grade::Pass {
            self.summary.passes -= 1;
        }
        self.summary.time = self.summary.time.saturating_sub(answer.time);
        Some(id)
    }

    /// Drops `card_id` without counting it as answered (suspended, deleted
    /// or moved away during the practice, or of a deck archived since).
    pub fn skip(&mut self, card_id: &str) {
        self.queue.retain(|id| id != card_id);
    }

    /// Brings back a card that [`Self::skip`] dropped (the Undo of a
    /// Suspend, a Delete or a Move): it is shown next. A card still queued
    /// stays where it is.
    pub fn put_back(&mut self, card_id: &str) {
        if !self.queue.iter().any(|id| id == card_id) {
            self.queue.push_front(card_id.to_owned());
        }
    }

    /// What was answered so far.
    #[must_use]
    pub fn summary(&self) -> Summary {
        self.summary
    }
}

#[cfg(test)]
mod tests;
