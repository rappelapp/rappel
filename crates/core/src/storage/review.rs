//! Recording reviews, and what a deck has to study.

use std::time::Duration;

use rusqlite::{Connection, TransactionBehavior, params};

use super::cards::live_card;
use super::decks::ensure_live_deck;
use super::schedule::{insert_event, read_card_state, read_log, rebuild_cards, write_card_state};
use super::settings::{scheduler_in, settings_in};
use super::{Store, observe_max_id};
use crate::domain::{CardState, EventKind, Grade, Phase, ReviewEvent};
use crate::error::{Error, Result};
use crate::id::IdGen;
use crate::scheduling::session::{Due, EARLY_MS, StudyInput, allowance};
use crate::scheduling::{Scheduler, surviving};

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
        let reviewed_at = next_entry_time(&tx, &mut self.ids, card_id, now)?;
        // Read inside the transaction, so a retention changed on another
        // connection is used as soon as it is committed (ADR 0027).
        let next = scheduler_in(&tx)?.schedule(card_id, &state, grade, reviewed_at)?;
        let event = ReviewEvent {
            id: self.ids.next(now)?,
            card_id: card_id.to_owned(),
            reviewed_at,
            kind: EventKind::Review(grade),
            duration_ms: i64::try_from(duration.as_millis()).unwrap_or(i64::MAX),
        };
        insert_event(&tx, &event)?;
        write_card_state(&tx, card_id, &next)?;
        tx.commit()?;
        Ok(event)
    }

    /// Resets card `card_id` at `now` (UTC milliseconds): appends a reset
    /// to its log and makes the card new, in one transaction (ADR 0028).
    /// Its history stays in the log; replaying it starts again from here.
    /// The entry is dated like a review ([`Self::record_review`]).
    ///
    /// # Errors
    /// [`Error::CardNotFound`](crate::Error::CardNotFound) for a missing or
    /// deleted card; id and SQLite errors. On error nothing is written.
    pub fn reset_card(&mut self, card_id: &str, now: i64) -> Result<ReviewEvent> {
        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let event = reset_in(&tx, &mut self.ids, card_id, now)?;
        tx.commit()?;
        Ok(event)
    }

    /// Cancels entry `event_id` of card `card_id` at `now`: appends an undo
    /// that names it, then rebuilds the card's cached schedule from its
    /// log, in one transaction (ADR 0028, #11). Only the card's latest
    /// review or reset that is not already undone can be cancelled, so a
    /// stale Undo button cannot reach further back; an undo is never
    /// undone. The undo is dated like a review ([`Self::record_review`]).
    ///
    /// # Errors
    /// [`Error::CardNotFound`](crate::Error::CardNotFound) for a missing or
    /// deleted card; [`Error::EventNotFound`](crate::Error::EventNotFound)
    /// if the entry is not in the card's log;
    /// [`Error::NotLatestEntry`](crate::Error::NotLatestEntry) if it is not
    /// the latest surviving review or reset; scheduler, id and SQLite
    /// errors. On error nothing is written.
    pub fn undo(&mut self, card_id: &str, event_id: &str, now: i64) -> Result<ReviewEvent> {
        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let scheduler = scheduler_in(&tx)?;
        let event = undo_in(&tx, &mut self.ids, &scheduler, card_id, event_id, now)?;
        tx.commit()?;
        Ok(event)
    }

    /// What deck `deck_id` has to study at `now`, for a
    /// [`Session`](crate::scheduling::session::Session). `day_start` is
    /// when "today" began (UTC milliseconds; the app computes the day start
    /// hour of the settings in local time). Read in one transaction, so the
    /// parts agree (ADR 0014):
    /// - due cards in review, most overdue first, at most
    ///   `max_reviews_per_day` less the older cards already reviewed today;
    /// - cards in the 10-minute step due within
    ///   `EARLY_MS`, earliest first;
    /// - how many cards of the deck had their first review today, deleted
    ///   ones included (they used today's budget);
    /// - how many older cards of the deck were reviewed today
    ///   (`reviews_today`), deleted ones included; a card failed before the
    ///   day start whose step falls after it counts too, though step cards
    ///   themselves are never held back;
    /// - the new cards still allowed today (`max_new_per_day`), oldest first.
    ///
    /// The limits are per deck and read from the settings in the same
    /// transaction (ADR 0027).
    ///
    /// # Errors
    /// [`Error::DeckNotFound`](crate::Error::DeckNotFound), SQLite errors.
    pub fn study_input(&self, deck_id: &str, now: i64, day_start: i64) -> Result<StudyInput> {
        let tx = self.conn.unchecked_transaction()?;
        let input = study_input_in(&tx, deck_id, now, day_start)?;
        // Read only: ending it commits nothing.
        tx.finish()?;
        Ok(input)
    }

    /// The first time after `now` (UTC milliseconds) at which
    /// [`Self::study_input`] for deck `deck_id` would offer one more
    /// reviewed card: a review card's `due_at`, or a step card's `due_at`
    /// minus `EARLY_MS`. `None` if no live card of the deck will become
    /// due. The day start, which renews the new-card budget, is the
    /// caller's to add.
    ///
    /// # Errors
    /// [`Error::DeckNotFound`](crate::Error::DeckNotFound), SQLite errors.
    pub fn next_study_change(&self, deck_id: &str, now: i64) -> Result<Option<i64>> {
        let tx = self.conn.unchecked_transaction()?;
        ensure_live_deck(&tx, deck_id)?;
        // Phases are our own constant strings, never user input.
        let next = tx.query_row(
            &format!(
                "SELECT min(CASE WHEN s.phase = '{review}' THEN s.due_at
                                 ELSE s.due_at - ?3 END)
                 FROM cards c JOIN card_schedule s ON s.card_id = c.id
                 WHERE c.deck_id = ?1 AND c.deleted_at IS NULL AND c.suspended = 0
                   AND (CASE WHEN s.phase = '{review}' THEN s.due_at
                             ELSE s.due_at - ?3 END) > ?2",
                review = Phase::Review.as_str()
            ),
            params![deck_id, now, EARLY_MS],
            |r| r.get(0),
        )?;
        tx.finish()?;
        Ok(next)
    }
}

/// [`Store::reset_card`] inside the caller's transaction.
pub(super) fn reset_in(
    tx: &Connection,
    ids: &mut IdGen,
    card_id: &str,
    now: i64,
) -> Result<ReviewEvent> {
    live_card(tx, card_id)?;
    let reviewed_at = next_entry_time(tx, ids, card_id, now)?;
    let event = ReviewEvent {
        id: ids.next(now)?,
        card_id: card_id.to_owned(),
        reviewed_at,
        kind: EventKind::Reset,
        duration_ms: 0,
    };
    insert_event(tx, &event)?;
    write_card_state(tx, card_id, &CardState::New)?;
    Ok(event)
}

/// [`Store::undo`] inside the caller's transaction.
pub(super) fn undo_in(
    tx: &Connection,
    ids: &mut IdGen,
    scheduler: &Scheduler,
    card_id: &str,
    event_id: &str,
    now: i64,
) -> Result<ReviewEvent> {
    live_card(tx, card_id)?;
    let log = read_log(tx, card_id)?;
    if !log.iter().any(|e| e.id == event_id) {
        return Err(Error::EventNotFound(event_id.to_owned()));
    }
    if surviving(&log)?.last().is_none_or(|e| e.id != event_id) {
        return Err(Error::NotLatestEntry(event_id.to_owned()));
    }
    let reviewed_at = next_entry_time(tx, ids, card_id, now)?;
    let event = ReviewEvent {
        id: ids.next(now)?,
        card_id: card_id.to_owned(),
        reviewed_at,
        kind: EventKind::Undo {
            undoes: event_id.to_owned(),
        },
        duration_ms: 0,
    };
    insert_event(tx, &event)?;
    // Rebuilt from the log, never patched: the result must equal a
    // replay of the log that never had the cancelled entry (#11).
    rebuild_cards(tx, scheduler, [card_id])?;
    Ok(event)
}

/// The time for a new entry of `card_id` written at `now`, inside the
/// write's transaction: `max(now, latest entry)`, so a clock that went
/// backwards cannot put it before the card's last entry. Also raises
/// the id generator above every entry, so the new id sorts after this
/// card's last entry and differs from every other one: after a
/// restart, the previous run may have issued `x + 1` for another card,
/// so observing only this card's `x` is not enough (ADR 0008).
fn next_entry_time(tx: &Connection, ids: &mut IdGen, card_id: &str, now: i64) -> Result<i64> {
    let last_at: Option<i64> = tx.query_row(
        "SELECT max(reviewed_at) FROM review_events WHERE card_id = ?1",
        [card_id],
        |r| r.get(0),
    )?;
    observe_max_id(tx, ids, "review_events.id")?;
    Ok(last_at.map_or(now, |last| now.max(last)))
}

/// The common table `counted(card_id, reviewed_at)`: the reviews of deck
/// `?1`'s cards (deleted ones included) that the daily limits count. Those
/// are the reviews no undo cancels, made after the card's last reset that
/// no undo cancels (ADR 0028): an undone review gives its place in today's
/// budget back, and a reset card is new again, as its rebuilt state is.
const COUNTED_REVIEWS: &str = "counted(card_id, reviewed_at) AS (
    SELECT e.card_id, e.reviewed_at FROM review_events e JOIN cards c ON c.id = e.card_id
    WHERE c.deck_id = ?1 AND e.kind = 'review'
      AND NOT EXISTS (SELECT 1 FROM review_events u WHERE u.undoes = e.id)
      AND NOT EXISTS (
          SELECT 1 FROM review_events r
          WHERE r.card_id = e.card_id AND r.kind = 'reset'
            AND (r.reviewed_at, r.id) > (e.reviewed_at, e.id)
            AND NOT EXISTS (SELECT 1 FROM review_events u WHERE u.undoes = r.id)))";

/// [`Store::study_input`] on `conn`, for callers that already hold a read
/// transaction. [`Store::list_decks`] counts with it too, so the deck list
/// and the deck page always agree.
pub(super) fn study_input_in(
    conn: &Connection,
    deck_id: &str,
    now: i64,
    day_start: i64,
) -> Result<StudyInput> {
    ensure_live_deck(conn, deck_id)?;
    let limits = settings_in(conn)?;
    // `limit` -1 is no limit in SQLite.
    let due = |phases: &str, until: i64, limit: i64| -> Result<Vec<Due>> {
        let mut stmt = conn.prepare(&format!(
            "SELECT c.id, s.due_at FROM cards c JOIN card_schedule s ON s.card_id = c.id
             WHERE c.deck_id = ?1 AND c.deleted_at IS NULL AND c.suspended = 0
               AND s.phase IN ({phases})
               AND s.due_at <= ?2
             ORDER BY s.due_at, c.seq LIMIT ?3"
        ))?;
        let rows = stmt.query_map(params![deck_id, until, limit], |r| {
            Ok(Due {
                card_id: r.get(0)?,
                due_at: r.get(1)?,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    };
    // Phases are our own constant strings, never user input.
    let quote = |p: Phase| format!("'{}'", p.as_str());
    // Cards that had a review before today and one today: not new today,
    // and not counted by `new_today`.
    let reviews_today: u32 = conn.query_row(
        &format!(
            "WITH {COUNTED_REVIEWS} SELECT count(*) FROM (
                 SELECT card_id FROM counted GROUP BY card_id
                 HAVING min(reviewed_at) < ?2 AND max(reviewed_at) >= ?2)"
        ),
        params![deck_id, day_start],
        |r| r.get(0),
    )?;
    let reviews = due(
        &quote(Phase::Review),
        now,
        i64::from(allowance(limits.max_reviews_per_day, reviews_today)),
    )?;
    // Step cards are never limited: a failed card always comes back.
    let steps = due(
        &format!("{}, {}", quote(Phase::Learning), quote(Phase::Relearning)),
        now.saturating_add(EARLY_MS),
        -1,
    )?;
    let new_today: u32 = conn.query_row(
        &format!(
            "WITH {COUNTED_REVIEWS} SELECT count(*) FROM (
                 SELECT card_id FROM counted GROUP BY card_id
                 HAVING min(reviewed_at) >= ?2)"
        ),
        params![deck_id, day_start],
        |r| r.get(0),
    )?;
    let mut stmt = conn.prepare(
        "SELECT c.id FROM cards c LEFT JOIN card_schedule s ON s.card_id = c.id
         WHERE c.deck_id = ?1 AND c.deleted_at IS NULL AND c.suspended = 0
           AND s.card_id IS NULL
         ORDER BY c.seq LIMIT ?2",
    )?;
    let new = stmt
        .query_map(
            params![deck_id, allowance(limits.max_new_per_day, new_today)],
            |r| r.get(0),
        )?
        .collect::<rusqlite::Result<_>>()?;
    drop(stmt);
    Ok(StudyInput {
        reviews,
        steps,
        new,
        new_today,
        reviews_today,
    })
}

#[cfg(test)]
mod tests;
