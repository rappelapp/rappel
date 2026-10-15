//! Recording reviews, and what a deck has to study.

use std::time::Duration;

use rusqlite::{TransactionBehavior, params};

use super::cards::live_card;
use super::decks::ensure_live_deck;
use super::schedule::{read_card_state, write_card_state};
use super::{Store, observe_max_id};
use crate::domain::{Grade, Phase, ReviewEvent};
use crate::error::Result;
use crate::scheduling::session::{Due, EARLY_MS, StudyInput, new_allowance};

impl Store {
    /// Records that card `card_id` was answered with `grade` after
    /// `duration` at time `now` (UTC milliseconds), and updates its cached
    /// schedule, in one transaction.
    ///
    /// The caller passes `now` so one clock serves the queue, the duration
    /// and the record (ADR 0014). The event is dated `max(now, last
    /// review)`, so a clock that went backwards cannot put it before the
    /// card's last review, and its id is greater than every event id in the
    /// log. Replaying the log therefore applies it last, and the cache stays
    /// equal to a rebuild (ADR 0008).
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

    /// What deck `deck_id` has to study at `now`, for a
    /// [`Session`](crate::scheduling::session::Session). `day_start` is
    /// when "today" began (UTC milliseconds; the app computes 4 a.m. local
    /// time). Read in one transaction, so the parts agree (ADR 0014):
    /// - due cards in review, most overdue first;
    /// - cards in the 10-minute step due within
    ///   [`EARLY_MS`], earliest first;
    /// - how many cards of the deck had their first review today, deleted
    ///   ones included (they used today's budget);
    /// - the new cards still allowed today, oldest first.
    ///
    /// # Errors
    /// [`Error::DeckNotFound`](crate::Error::DeckNotFound), SQLite errors.
    pub fn study_input(&self, deck_id: &str, now: i64, day_start: i64) -> Result<StudyInput> {
        let tx = self.conn.unchecked_transaction()?;
        ensure_live_deck(&tx, deck_id)?;
        let due = |phases: &str, until: i64| -> Result<Vec<Due>> {
            let mut stmt = tx.prepare(&format!(
                "SELECT c.id, s.due_at FROM cards c JOIN card_schedule s ON s.card_id = c.id
                 WHERE c.deck_id = ?1 AND c.deleted_at IS NULL AND s.phase IN ({phases})
                   AND s.due_at <= ?2
                 ORDER BY s.due_at, c.seq"
            ))?;
            let rows = stmt.query_map(params![deck_id, until], |r| {
                Ok(Due {
                    card_id: r.get(0)?,
                    due_at: r.get(1)?,
                })
            })?;
            Ok(rows.collect::<rusqlite::Result<_>>()?)
        };
        // Phases are our own constant strings, never user input.
        let quote = |p: Phase| format!("'{}'", p.as_str());
        let reviews = due(&quote(Phase::Review), now)?;
        let steps = due(
            &format!("{}, {}", quote(Phase::Learning), quote(Phase::Relearning)),
            now.saturating_add(EARLY_MS),
        )?;
        let new_today: u32 = tx.query_row(
            "SELECT count(*) FROM (
                 SELECT e.card_id FROM review_events e JOIN cards c ON c.id = e.card_id
                 WHERE c.deck_id = ?1
                 GROUP BY e.card_id HAVING min(e.reviewed_at) >= ?2)",
            params![deck_id, day_start],
            |r| r.get(0),
        )?;
        let mut stmt = tx.prepare(
            "SELECT c.id FROM cards c LEFT JOIN card_schedule s ON s.card_id = c.id
             WHERE c.deck_id = ?1 AND c.deleted_at IS NULL AND s.card_id IS NULL
             ORDER BY c.seq LIMIT ?2",
        )?;
        let new = stmt
            .query_map(params![deck_id, new_allowance(new_today)], |r| r.get(0))?
            .collect::<rusqlite::Result<_>>()?;
        drop(stmt);
        // Read only: ending it commits nothing.
        tx.finish()?;
        Ok(StudyInput {
            reviews,
            steps,
            new,
            new_today,
        })
    }
}

#[cfg(test)]
mod tests;
