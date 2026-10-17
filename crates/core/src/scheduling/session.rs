//! A study session: which card to show next, and what was answered.
//! Pure: no database and no clock. The caller passes `now` and the state
//! each answer produced (ADR 0014).
//!
//! Order of cards:
//! 1. a card in the 10-minute step that is due now (earliest first);
//! 2. otherwise the next card of the queue: due reviews with new cards
//!    spread evenly among them (`interleave`);
//! 3. otherwise a step card due within `EARLY_MS`, shown early;
//! 4. otherwise nothing: the session is over.
//!
//! A card answered into the step (a new card, or any Fail) comes back in
//! the same session.

use std::collections::HashSet;
use std::time::Duration;

use crate::domain::{CardState, Grade, Phase};

/// How early a step card may be shown when nothing else is left.
pub(crate) const EARLY_MS: i64 = 20 * 60 * 1000;

/// The longest time recorded for one answer. A card left on screen while
/// the user is away should not count as an hour of study.
pub(crate) const MAX_DURATION: Duration = Duration::from_secs(60);

/// A card and when it is due, UTC milliseconds.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Due {
    /// The card's id.
    pub card_id: String,
    /// When it is due.
    pub due_at: i64,
}

/// What a deck has to study at one moment (see
/// [`Store::study_input`](crate::storage::Store::study_input)).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct StudyInput {
    /// Cards in review that are due, most overdue first, cut to the reviews
    /// still allowed today.
    pub reviews: Vec<Due>,
    /// Cards in the 10-minute step due within `EARLY_MS`, earliest first.
    pub steps: Vec<Due>,
    /// New cards still allowed today, oldest first.
    pub new: Vec<String>,
    /// New cards this deck already introduced today.
    pub new_today: u32,
    /// Older cards of this deck already reviewed today.
    pub reviews_today: u32,
}

impl StudyInput {
    /// Whether there is nothing to study.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.reviews.is_empty() && self.steps.is_empty() && self.new.is_empty()
    }
}

/// How many more cards may be taken today under a daily `limit`, after
/// `done_today` (new cards introduced, or older cards reviewed).
#[must_use]
pub(crate) fn allowance(limit: u32, done_today: u32) -> u32 {
    limit.saturating_sub(done_today)
}

/// `duration`, capped at `MAX_DURATION`.
#[must_use]
pub fn capped(duration: Duration) -> Duration {
    duration.min(MAX_DURATION)
}

/// Merges `reviews` and `new`, keeping the order of each and spreading the
/// new cards evenly. Review `j` (of `R`) sits at `(j + 1) / (R + 1)` and new
/// card `i` (of `N`) at `(i + 1) / (N + 1)`; on a tie the review goes first.
/// With `R >= N`, no two new cards are next to each other.
#[must_use]
pub(crate) fn interleave<T>(reviews: Vec<T>, new: Vec<T>) -> Vec<T> {
    let (r_len, n_len) = (reviews.len() as u128, new.len() as u128);
    let mut out = Vec::with_capacity(reviews.len() + new.len());
    let mut reviews = reviews.into_iter().peekable();
    let mut new = new.into_iter().peekable();
    let (mut j, mut i) = (0_u128, 0_u128);
    loop {
        let review_first = match (reviews.peek(), new.peek()) {
            (None, None) => break,
            (Some(_), None) => true,
            (None, Some(_)) => false,
            // (j+1)/(R+1) <= (i+1)/(N+1), exactly, as cross-products.
            (Some(_), Some(_)) => (j + 1) * (n_len + 1) <= (i + 1) * (r_len + 1),
        };
        if review_first {
            out.extend(reviews.next());
            j += 1;
        } else {
            out.extend(new.next());
            i += 1;
        }
    }
    out
}

/// What a session did, for its summary.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Summary {
    /// Distinct cards answered.
    pub cards: usize,
    /// Answers given (a step card answered twice counts twice).
    pub answers: usize,
    /// Answers that were Pass.
    pub passes: usize,
    /// Sum of the recorded (capped) durations.
    pub time: Duration,
}

impl Summary {
    /// Passes as a rounded percentage of answers; `None` without answers.
    #[must_use]
    pub fn pass_percent(&self) -> Option<usize> {
        (self.answers > 0).then(|| (self.passes * 100 + self.answers / 2) / self.answers)
    }
}

/// What one [`Session::graded`] changed, kept so [`Session::undo_last`] can
/// put it back exactly.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Answer {
    card_id: String,
    /// The log entry the answer was recorded as.
    entry_id: String,
    /// Where the card was in the queue, if it was there.
    queue_pos: Option<usize>,
    /// The step the card was waiting in, if it was there.
    removed_step: Option<Due>,
    /// Whether the card was answered for the first time in this session.
    newly_seen: bool,
    grade: Grade,
    /// The capped duration added to the summary.
    time: Duration,
}

/// One study session of one deck.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Session {
    /// Due reviews and new cards, in showing order.
    queue: std::collections::VecDeque<String>,
    /// Step cards, sorted by `(due_at, card_id)`.
    steps: Vec<Due>,
    seen: HashSet<String>,
    summary: Summary,
    /// The answers that can still be undone, oldest first.
    answers: Vec<Answer>,
    /// A card an undo brought back: it is shown next, whatever is due.
    pinned: Option<String>,
}

impl Session {
    /// A session over `input`.
    #[must_use]
    pub fn new(input: StudyInput) -> Self {
        let reviews = input.reviews.into_iter().map(|d| d.card_id).collect();
        let mut session = Self {
            queue: interleave(reviews, input.new).into(),
            steps: input.steps,
            seen: HashSet::new(),
            summary: Summary::default(),
            answers: Vec::new(),
            pinned: None,
        };
        session.sort_steps();
        session
    }

    /// The card to show at `now`, or `None` when the session is over. It
    /// stays the answer until [`Self::graded`] is called for it.
    #[must_use]
    pub fn next(&self, now: i64) -> Option<&str> {
        if let Some(card) = &self.pinned {
            return Some(card);
        }
        let step = self.steps.first();
        if let Some(step) = step.filter(|s| s.due_at <= now) {
            return Some(&step.card_id);
        }
        if let Some(card) = self.queue.front() {
            return Some(card);
        }
        step.filter(|s| s.due_at <= now.saturating_add(EARLY_MS))
            .map(|s| s.card_id.as_str())
    }

    /// Cards still waiting: the queue and the step cards (including ones
    /// that may come too late for this session).
    #[must_use]
    pub fn remaining(&self) -> usize {
        self.queue.len() + self.steps.len()
    }

    /// Records that `card_id` was answered with `grade` after `duration`,
    /// leading to `state` (as stored by `record_review`, which made log
    /// entry `entry_id`). A card left in the step comes back later in the
    /// session. The answer can be taken back with [`Self::undo_last`].
    pub fn graded(
        &mut self,
        card_id: &str,
        entry_id: &str,
        grade: Grade,
        duration: Duration,
        state: &CardState,
    ) {
        let queue_pos = self.queue.iter().position(|id| id == card_id);
        let removed_step = self
            .steps
            .iter()
            .position(|d| d.card_id == card_id)
            .map(|i| self.steps.remove(i));
        self.queue.retain(|id| id != card_id);
        if self.pinned.as_deref() == Some(card_id) {
            self.pinned = None;
        }
        if let CardState::Scheduled(s) = state
            && matches!(s.phase, Phase::Learning | Phase::Relearning)
        {
            self.steps.push(Due {
                card_id: card_id.to_owned(),
                due_at: s.due_at,
            });
            self.sort_steps();
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
            entry_id: entry_id.to_owned(),
            queue_pos,
            removed_step,
            newly_seen,
            grade,
            time,
        });
    }

    /// The card and log entry of the answer [`Self::undo_last`] would take
    /// back, or `None` when there is nothing to undo.
    #[must_use]
    pub fn last_answer(&self) -> Option<(&str, &str)> {
        self.answers
            .last()
            .map(|a| (a.card_id.as_str(), a.entry_id.as_str()))
    }

    /// Takes back the newest answer, after the caller undid it in the log:
    /// the card returns where it was, the summary loses it, and the card is
    /// the next one shown. Returns its id; `None` when there is nothing to
    /// undo. Answers are undone newest first, so each inverse is exact.
    pub fn undo_last(&mut self) -> Option<String> {
        let answer = self.answers.pop()?;
        let id = answer.card_id;
        // The step an answer into the learning steps added.
        self.steps.retain(|d| d.card_id != id);
        // Not twice: `put_back` may have queued the card again since.
        if let Some(pos) = answer.queue_pos
            && !self.queue.contains(&id)
        {
            self.queue.insert(pos.min(self.queue.len()), id.clone());
        }
        if let Some(step) = answer.removed_step {
            self.steps.push(step);
            self.sort_steps();
        }
        if answer.newly_seen {
            self.seen.remove(&id);
            self.summary.cards -= 1;
        }
        self.summary.answers -= 1;
        if answer.grade == Grade::Pass {
            self.summary.passes -= 1;
        }
        self.summary.time = self.summary.time.saturating_sub(answer.time);
        self.pinned = Some(id.clone());
        Some(id)
    }

    /// Forgets every answer that could be undone, e.g. once the log says
    /// the newest can no longer be.
    pub fn forget_answers(&mut self) {
        self.answers.clear();
    }

    /// Drops `card_id` from the session without counting it as answered,
    /// e.g. when it could not be read.
    pub fn skip(&mut self, card_id: &str) {
        if self.pinned.as_deref() == Some(card_id) {
            self.pinned = None;
        }
        self.queue.retain(|id| id != card_id);
        self.steps.retain(|d| d.card_id != card_id);
    }

    /// Brings back a card that [`Self::skip`] dropped, e.g. after the Undo
    /// of a suspend or a delete: it goes to the front of the queue, so it
    /// comes next, except for a card an undo brought back and a step card
    /// that is already due, which still come first. A card that was waiting
    /// in a step comes back through the queue, so it may be shown before its
    /// step is due. A card the
    /// session already has (queued, waiting in a step, or the one an undo
    /// brought back) stays where it is, so calling this twice is harmless.
    pub fn put_back(&mut self, card_id: &str) {
        let held = self.pinned.as_deref() == Some(card_id)
            || self.queue.iter().any(|id| id == card_id)
            || self.steps.iter().any(|d| d.card_id == card_id);
        if !held {
            self.queue.push_front(card_id.to_owned());
        }
    }

    /// What was answered so far.
    #[must_use]
    pub fn summary(&self) -> Summary {
        self.summary
    }

    fn sort_steps(&mut self) {
        self.steps
            .sort_by(|a, b| (a.due_at, &a.card_id).cmp(&(b.due_at, &b.card_id)));
    }
}

#[cfg(test)]
mod tests;
