//! Recording reviews, and the due and new queues of a deck.

use std::time::Duration;

use rusqlite::{TransactionBehavior, params};

use super::cards::{CARD_COLUMNS, card_from_row, live_card};
use super::decks::ensure_live_deck;
use super::schedule::{read_card_state, write_card_state};
use super::{Store, observe_max_id};
use crate::domain::{Card, Grade, ReviewEvent};
use crate::error::Result;
use crate::time::now_ms;

impl Store {
    /// Records that card `card_id` was answered with `grade` after
    /// `duration`, and updates its cached schedule, in one transaction.
    ///
    /// The event is dated `max(now, last review)`, so a clock that went
    /// backwards cannot put it before the card's last review, and its id is
    /// greater than every event id in the log. Replaying the log therefore applies
    /// it last, and the cache stays equal to a rebuild (ADR 0008).
    ///
    /// # Errors
    /// [`Error::CardNotFound`](crate::Error::CardNotFound) for a missing or
    /// deleted card; scheduler, id and SQLite errors. On error nothing is
    /// written.
    pub fn record_review(
        &mut self,
        card_id: &str,
        grade: Grade,
        duration: Duration,
    ) -> Result<ReviewEvent> {
        self.record_review_at(card_id, grade, duration, now_ms())
    }

    /// [`Self::record_review`] at time `now` (UTC milliseconds).
    pub(super) fn record_review_at(
        &mut self,
        card_id: &str,
        grade: Grade,
        duration: Duration,
        now: i64,
    ) -> Result<ReviewEvent> {
        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        live_card(&tx, card_id)?;
        let state = read_card_state(&tx, card_id)?;
        let last_at: Option<i64> = tx.query_row(
            "SELECT max(reviewed_at) FROM review_events WHERE card_id = ?1",
            [card_id],
            |r| r.get(0),
        )?;
        let reviewed_at = last_at.map_or(now, |last| now.max(last));
        // The new id must sort after this card's last event. It must also
        // differ from every other event's id: after a restart, the previous
        // run may have issued `x + 1` for another card, so observing only
        // this card's `x` is not enough. The largest id covers both.
        observe_max_id(&tx, &mut self.ids, "review_events.id")?;
        let next = self
            .scheduler
            .schedule(card_id, &state, grade, reviewed_at)?;
        let event = ReviewEvent {
            id: self.ids.next(now)?,
            card_id: card_id.to_owned(),
            reviewed_at,
            grade,
            duration_ms: i64::try_from(duration.as_millis()).unwrap_or(i64::MAX),
        };
        tx.execute(
            "INSERT INTO review_events (id, card_id, reviewed_at, grade, duration_ms)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![
                event.id,
                event.card_id,
                event.reviewed_at,
                event.grade.as_str(),
                event.duration_ms
            ],
        )?;
        write_card_state(&tx, card_id, &next)?;
        tx.commit()?;
        Ok(event)
    }

    /// The live cards of deck `deck_id` that have been reviewed and are due
    /// at `now` (UTC milliseconds), most overdue first. Daily limits are
    /// not applied.
    ///
    /// # Errors
    /// [`Error::DeckNotFound`](crate::Error::DeckNotFound), SQLite errors.
    pub fn due_cards(&self, deck_id: &str, now: i64) -> Result<Vec<Card>> {
        ensure_live_deck(&self.conn, deck_id)?;
        let mut stmt = self.conn.prepare(&format!(
            "SELECT {CARD_COLUMNS} FROM cards c JOIN card_schedule s ON s.card_id = c.id
             WHERE c.deck_id = ?1 AND c.deleted_at IS NULL AND s.due_at <= ?2
             ORDER BY s.due_at, c.seq"
        ))?;
        let rows = stmt.query_map(params![deck_id, now], card_from_row)?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    /// Up to `limit` live cards of deck `deck_id` that were never reviewed,
    /// oldest first.
    ///
    /// # Errors
    /// [`Error::DeckNotFound`](crate::Error::DeckNotFound), SQLite errors.
    pub fn new_cards(&self, deck_id: &str, limit: u32) -> Result<Vec<Card>> {
        ensure_live_deck(&self.conn, deck_id)?;
        let mut stmt = self.conn.prepare(&format!(
            "SELECT {CARD_COLUMNS} FROM cards c LEFT JOIN card_schedule s ON s.card_id = c.id
             WHERE c.deck_id = ?1 AND c.deleted_at IS NULL AND s.card_id IS NULL
             ORDER BY c.seq LIMIT ?2"
        ))?;
        let rows = stmt.query_map(params![deck_id, limit], card_from_row)?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }
}

#[cfg(test)]
mod tests;
