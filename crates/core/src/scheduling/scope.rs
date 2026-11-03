//! What a study session covers (a deck, or a tag across decks), and how a
//! tag session takes its cards from the decks: pure, over plain data
//! (ADR 0069).
//!
//! The daily limits stay per deck (ADR 0027). A tag session takes, from
//! each deck, the due and new cards carrying the tag within *that deck's*
//! allowance left today, then merges them with the order rules of a deck
//! session: due reviews most overdue first, step cards earliest first, new
//! cards oldest first; [`Session`](super::Session) interleaves them. The
//! log is per card, so a card studied in a tag session counts toward its
//! deck's "done today" like any other review.

use std::collections::HashMap;

use super::session::{Due, EARLY_MS, StudyInput};

/// What a study session covers.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum StudyScope {
    /// The cards of one deck, by id.
    Deck(String),
    /// The cards carrying one tag, by tag id, in every live deck that is
    /// not archived (ADR 0068, 0069).
    Tag(String),
}

impl StudyScope {
    /// The scope of deck `id`.
    #[must_use]
    pub fn deck(id: impl Into<String>) -> Self {
        Self::Deck(id.into())
    }

    /// The scope of tag `id`.
    #[must_use]
    pub fn tag(id: impl Into<String>) -> Self {
        Self::Tag(id.into())
    }
}

/// One deck's limits left today, for [`allocate`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct DeckAllowance {
    /// The deck's id.
    pub deck_id: String,
    /// New cards it may still introduce today.
    pub new: u32,
    /// Older cards it may still review today.
    pub reviews: u32,
    /// New cards it introduced today.
    pub new_today: u32,
    /// Older cards of it reviewed today.
    pub reviews_today: u32,
}

/// Where a candidate card is in its schedule.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Stage {
    /// Never reviewed (or reset).
    New,
    /// In review, due at this time.
    Review(i64),
    /// In the 10-minute step (learning or relearning), due at this time.
    Step(i64),
}

/// A card that may be studied in a scoped session, for [`allocate`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Candidate {
    /// The card's id.
    pub card_id: String,
    /// Its deck's id.
    pub deck_id: String,
    /// Its creation order (`cards.seq`): "oldest first", and the tie
    /// breaker of equal due times, as in a deck session.
    pub seq: i64,
    /// Suspended cards are never studied (ADR 0035).
    pub suspended: bool,
    /// Deleted cards are never studied.
    pub deleted: bool,
    /// Where it is in its schedule.
    pub stage: Stage,
}

/// The study input of a scoped session at `now`: from each deck of
/// `allowances`, its candidates that are live, not suspended and due (a
/// review due by `now`, a step card due by `now + EARLY_MS`, or new),
/// cut per deck to that deck's allowance of reviews and of new cards (step
/// cards are never limited), keeping the deck's own order; then merged:
/// reviews and steps by `(due_at, seq)`, new cards by `seq`.
///
/// A candidate whose deck is not in `allowances` (an archived or deleted
/// deck, which the caller leaves out) is never taken. `new_today` and
/// `reviews_today` are the sums over the decks that gave a candidate.
#[must_use]
pub(crate) fn allocate(
    allowances: &[DeckAllowance],
    candidates: Vec<Candidate>,
    now: i64,
) -> StudyInput {
    let mut decks: HashMap<&str, DeckPart> = allowances
        .iter()
        .map(|a| (a.deck_id.as_str(), DeckPart::default()))
        .collect();
    for card in candidates {
        if card.suspended || card.deleted {
            continue;
        }
        let Some(part) = decks.get_mut(card.deck_id.as_str()) else {
            continue;
        };
        match card.stage {
            Stage::New => part.new.push((card.seq, card.card_id)),
            Stage::Review(due_at) if due_at <= now => {
                part.reviews.push((due_at, card.seq, card.card_id));
            }
            Stage::Step(due_at) if due_at <= now.saturating_add(EARLY_MS) => {
                part.steps.push((due_at, card.seq, card.card_id));
            }
            Stage::Review(_) | Stage::Step(_) => {}
        }
    }
    let mut reviews = Vec::new();
    let mut steps = Vec::new();
    let mut new = Vec::new();
    let (mut new_today, mut reviews_today) = (0_u32, 0_u32);
    for allowance in allowances {
        let Some(mut part) = decks.remove(allowance.deck_id.as_str()) else {
            // The same deck twice: its cards were taken the first time.
            continue;
        };
        if part.is_empty() {
            continue;
        }
        new_today = new_today.saturating_add(allowance.new_today);
        reviews_today = reviews_today.saturating_add(allowance.reviews_today);
        part.reviews.sort_unstable();
        part.new.sort_unstable();
        reviews.extend(part.reviews.into_iter().take(limit(allowance.reviews)));
        new.extend(part.new.into_iter().take(limit(allowance.new)));
        steps.extend(part.steps);
    }
    reviews.sort_unstable();
    steps.sort_unstable();
    new.sort_unstable();
    let due = |(due_at, _, card_id): (i64, i64, String)| Due { card_id, due_at };
    StudyInput {
        reviews: reviews.into_iter().map(due).collect(),
        steps: steps.into_iter().map(due).collect(),
        new: new.into_iter().map(|(_, id)| id).collect(),
        new_today,
        reviews_today,
    }
}

/// An allowance as a count of items to take.
fn limit(allowance: u32) -> usize {
    usize::try_from(allowance).unwrap_or(usize::MAX)
}

/// One deck's due candidates, before its limits are applied.
#[derive(Debug, Default)]
struct DeckPart {
    /// `(due_at, seq, card_id)`.
    reviews: Vec<(i64, i64, String)>,
    /// `(due_at, seq, card_id)`.
    steps: Vec<(i64, i64, String)>,
    /// `(seq, card_id)`.
    new: Vec<(i64, String)>,
}

impl DeckPart {
    fn is_empty(&self) -> bool {
        self.reviews.is_empty() && self.steps.is_empty() && self.new.is_empty()
    }
}

#[cfg(test)]
mod tests;
