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

/// New cards a deck introduces per day (SPEC §3 default; a setting later).
pub(crate) const NEW_PER_DAY: u32 = 20;

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
    /// Cards in review that are due, most overdue first.
    pub reviews: Vec<Due>,
    /// Cards in the 10-minute step due within `EARLY_MS`, earliest first.
    pub steps: Vec<Due>,
    /// New cards still allowed today, oldest first.
    pub new: Vec<String>,
    /// New cards this deck already introduced today.
    pub new_today: u32,
}

impl StudyInput {
    /// Whether there is nothing to study.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.reviews.is_empty() && self.steps.is_empty() && self.new.is_empty()
    }
}

/// How many new cards may still be introduced today, after `new_today`.
#[must_use]
pub(crate) fn new_allowance(new_today: u32) -> u32 {
    NEW_PER_DAY.saturating_sub(new_today)
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

/// One study session of one deck.
#[derive(Debug, Clone)]
pub struct Session {
    /// Due reviews and new cards, in showing order.
    queue: std::collections::VecDeque<String>,
    /// Step cards, sorted by `(due_at, card_id)`.
    steps: Vec<Due>,
    seen: HashSet<String>,
    summary: Summary,
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
        };
        session.sort_steps();
        session
    }

    /// The card to show at `now`, or `None` when the session is over. It
    /// stays the answer until [`Self::graded`] is called for it.
    #[must_use]
    pub fn next(&self, now: i64) -> Option<&str> {
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
    /// leading to `state` (as stored by `record_review`). A card left in
    /// the step comes back later in the session.
    pub fn graded(&mut self, card_id: &str, grade: Grade, duration: Duration, state: &CardState) {
        self.queue.retain(|id| id != card_id);
        self.steps.retain(|d| d.card_id != card_id);
        if let CardState::Scheduled(s) = state
            && matches!(s.phase, Phase::Learning | Phase::Relearning)
        {
            self.steps.push(Due {
                card_id: card_id.to_owned(),
                due_at: s.due_at,
            });
            self.sort_steps();
        }
        if self.seen.insert(card_id.to_owned()) {
            self.summary.cards += 1;
        }
        self.summary.answers += 1;
        if grade == Grade::Pass {
            self.summary.passes += 1;
        }
        self.summary.time += capped(duration);
    }

    /// Drops `card_id` from the session without counting it as answered,
    /// e.g. when it could not be read.
    pub fn skip(&mut self, card_id: &str) {
        self.queue.retain(|id| id != card_id);
        self.steps.retain(|d| d.card_id != card_id);
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
