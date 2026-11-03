//! Recording reviews, and what a deck has to study.

use std::time::Duration;

use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};

use super::cards::live_card;
use super::decks::{ensure_active_deck, ensure_live_deck};
use super::schedule::{insert_event, read_card_state, read_log, rebuild_cards, write_card_state};
use super::settings::{scheduler_in, settings_in};
use super::{Store, observe_max_id};
use crate::domain::{CardState, EventKind, Grade, Phase, ReviewEvent};
use crate::error::{Error, Result};
use crate::id::IdGen;
use crate::scheduling::StudyScope;
use crate::scheduling::scope::{Candidate, DeckAllowance, Stage, allocate};
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
    /// deleted card; [`Error::CardAlreadyNew`](crate::Error::CardAlreadyNew)
    /// if the card has no surviving review since it was created or last
    /// reset; id and SQLite errors. On error nothing is written.
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

    /// What `scope` has to study at `now`, for a
    /// [`Session`](crate::scheduling::session::Session). `day_start` is
    /// when "today" began (UTC milliseconds; the app computes the day start
    /// hour of the settings in local time). Read in one transaction, so the
    /// parts agree (ADR 0014).
    ///
    /// For a deck ([`StudyScope::Deck`]):
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
    /// For a tag ([`StudyScope::Tag`], ADR 0069): the same, over the live
    /// cards carrying the tag in every live deck that is not archived, each
    /// deck giving at most what is left of *its own* limits today; the
    /// parts are merged in the same order (`scope::allocate`), and
    /// `new_today` and `reviews_today` are summed over the decks that gave
    /// a card. A tag on no such card gives nothing to study.
    ///
    /// # Errors
    /// [`Error::DeckNotFound`](crate::Error::DeckNotFound),
    /// [`Error::DeckArchived`](crate::Error::DeckArchived) (an archived
    /// deck is not studied, ADR 0068),
    /// [`Error::TagNotFound`](crate::Error::TagNotFound), SQLite errors.
    pub fn study_input(&self, scope: &StudyScope, now: i64, day_start: i64) -> Result<StudyInput> {
        let tx = self.conn.unchecked_transaction()?;
        let input = match scope {
            StudyScope::Deck(deck_id) => study_input_in(&tx, deck_id, now, day_start)?,
            StudyScope::Tag(tag_id) => tag_study_input_in(&tx, tag_id, now, day_start)?,
        };
        // Read only: ending it commits nothing.
        tx.finish()?;
        Ok(input)
    }

    /// The first time after `now` (UTC milliseconds) at which
    /// [`Self::study_input`] for `scope` would offer one more
    /// reviewed card: a review card's `due_at`, or a step card's `due_at`
    /// minus `EARLY_MS`. `None` if no live card of the scope will become
    /// due. The day start, which renews the new-card budget, is the
    /// caller's to add.
    ///
    /// # Errors
    /// [`Error::DeckNotFound`](crate::Error::DeckNotFound),
    /// [`Error::TagNotFound`](crate::Error::TagNotFound), SQLite errors.
    pub fn next_study_change(&self, scope: &StudyScope, now: i64) -> Result<Option<i64>> {
        let tx = self.conn.unchecked_transaction()?;
        let next = match scope {
            StudyScope::Deck(deck_id) => {
                ensure_live_deck(&tx, deck_id)?;
                next_study_change_in(&tx, deck_id, now)?
            }
            StudyScope::Tag(tag_id) => {
                ensure_tag(&tx, tag_id)?;
                tag_next_study_change_in(&tx, tag_id, now)?
            }
        };
        tx.finish()?;
        Ok(next)
    }
}

/// [`Store::next_study_change`] on a connection inside a read transaction,
/// for a deck already known to be live. [`deck_counts_in`] computes the
/// same value in its aggregate pass, with the same conditions.
fn next_study_change_in(conn: &Connection, deck_id: &str, now: i64) -> Result<Option<i64>> {
    // Phases are our own constant strings, never user input.
    Ok(conn.query_row(
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
    )?)
}

/// [`Store::reset_card`] inside the caller's transaction.
pub(super) fn reset_in(
    tx: &Connection,
    ids: &mut IdGen,
    card_id: &str,
    now: i64,
) -> Result<ReviewEvent> {
    live_card(tx, card_id)?;
    // The cache has no row exactly when no review survives since the last
    // surviving reset: a reset and an undo both rewrite it.
    if matches!(read_card_state(tx, card_id)?, CardState::New) {
        return Err(Error::CardAlreadyNew(card_id.to_owned()));
    }
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
///
/// Each card's last surviving reset is found once (`last_reset`, a window
/// over the deck's resets), and every review is compared with it. Asking
/// "is there a later surviving reset?" for each review instead scanned
/// every later entry of the card when there was none: quadratic in the
/// length of a card's log, 7 s of CPU for one card of 8,000 reviews in a
/// debug build (ADR 0050).
const COUNTED_REVIEWS: &str = "last_reset(card_id, reviewed_at, id) AS (
    SELECT card_id, reviewed_at, id FROM (
        SELECT r.card_id, r.reviewed_at, r.id,
               row_number() OVER (PARTITION BY r.card_id
                                  ORDER BY r.reviewed_at DESC, r.id DESC) AS latest
        FROM review_events r JOIN cards c ON c.id = r.card_id
        WHERE c.deck_id = ?1 AND r.kind = 'reset'
          AND NOT EXISTS (SELECT 1 FROM review_events u WHERE u.undoes = r.id))
    WHERE latest = 1),
counted(card_id, reviewed_at) AS (
    SELECT e.card_id, e.reviewed_at FROM review_events e JOIN cards c ON c.id = e.card_id
    LEFT JOIN last_reset l ON l.card_id = e.card_id
    WHERE c.deck_id = ?1 AND e.kind = 'review'
      AND NOT EXISTS (SELECT 1 FROM review_events u WHERE u.undoes = e.id)
      AND (l.card_id IS NULL OR (e.reviewed_at, e.id) > (l.reviewed_at, l.id)))";

/// [`Store::study_input`] on `conn`, for callers that already hold a read
/// transaction. [`Store::list_decks`] counts with it too, so the deck list
/// and the deck page always agree.
pub(super) fn study_input_in(
    conn: &Connection,
    deck_id: &str,
    now: i64,
    day_start: i64,
) -> Result<StudyInput> {
    ensure_active_deck(conn, deck_id)?;
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
    let (reviews_today, new_today) = today_counts_in(conn, deck_id, day_start)?;
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

/// How many older cards of deck `deck_id` were reviewed since `day_start`
/// (`reviews_today`), and how many had their first review since then
/// (`new_today`), in one pass over the counted reviews. Deleted cards
/// count: they used today's budget.
fn today_counts_in(conn: &Connection, deck_id: &str, day_start: i64) -> Result<(u32, u32)> {
    // A card with reviews before today and today is not new today. `sum`
    // of no rows is NULL, hence the `coalesce`.
    let (reviews_today, new_today): (i64, i64) = conn.query_row(
        &format!(
            "WITH {COUNTED_REVIEWS} SELECT
                 coalesce(sum(first < ?2 AND last >= ?2), 0),
                 coalesce(sum(first >= ?2), 0)
             FROM (SELECT min(reviewed_at) AS first, max(reviewed_at) AS last
                   FROM counted GROUP BY card_id)"
        ),
        params![deck_id, day_start],
        |r| Ok((r.get(0)?, r.get(1)?)),
    )?;
    let count = |n: i64| u32::try_from(n.max(0)).unwrap_or(u32::MAX);
    Ok((count(reviews_today), count(new_today)))
}

/// [`Error::TagNotFound`] unless tag `tag_id` exists. Tags are never
/// deleted (ADR 0029), so this fails only for an id that never was one.
pub(super) fn ensure_tag(conn: &Connection, tag_id: &str) -> Result<()> {
    conn.query_row("SELECT 1 FROM tags WHERE id = ?1", [tag_id], |_| Ok(()))
        .optional()?
        .ok_or_else(|| Error::TagNotFound(tag_id.to_owned()))
}

/// The tables of a tag session's cards: `ct` (the card tags), `c` (the
/// cards) and `d` (their decks), to be filtered by [`TAGGED_IN_SCOPE`].
pub(super) const TAGGED_FROM: &str = "card_tags ct
    JOIN cards c ON c.id = ct.card_id
    JOIN decks d ON d.id = c.deck_id";

/// The cards carrying tag `?1` in every live deck that is not archived
/// (ADR 0068: the tag scope leaves archived decks out), deleted and
/// suspended cards included. Shared by the queries of a tag session, so
/// they cannot disagree on its decks.
pub(super) const TAGGED_IN_SCOPE: &str =
    "ct.tag_id = ?1 AND d.deleted_at IS NULL AND d.archived_at IS NULL";

/// [`Store::study_input`] of tag `tag_id`, inside the caller's read
/// transaction: each deck that has a live card with the tag gives its
/// allowance left today (as [`study_input_in`] computes it), every card
/// with the tag is a candidate, and `scope::allocate` cuts and merges them
/// (ADR 0069).
fn tag_study_input_in(
    conn: &Connection,
    tag_id: &str,
    now: i64,
    day_start: i64,
) -> Result<StudyInput> {
    ensure_tag(conn, tag_id)?;
    let limits = settings_in(conn)?;
    let deck_ids: Vec<String> = {
        let mut stmt = conn.prepare(&format!(
            "SELECT DISTINCT c.deck_id FROM {TAGGED_FROM}
             WHERE {TAGGED_IN_SCOPE} AND c.deleted_at IS NULL
             ORDER BY c.deck_id"
        ))?;
        let rows = stmt.query_map([tag_id], |r| r.get(0))?;
        rows.collect::<rusqlite::Result<_>>()?
    };
    let mut allowances = Vec::with_capacity(deck_ids.len());
    for deck_id in deck_ids {
        let (reviews_today, new_today) = today_counts_in(conn, &deck_id, day_start)?;
        allowances.push(DeckAllowance {
            deck_id,
            new: allowance(limits.max_new_per_day, new_today),
            reviews: allowance(limits.max_reviews_per_day, reviews_today),
            new_today,
            reviews_today,
        });
    }
    // Phases are our own constant strings, never user input. A phase
    // that is none of them (the schema allows none) is not studied.
    let mut stmt = conn.prepare(&format!(
        "SELECT c.id, c.deck_id, c.seq, c.suspended <> 0, c.deleted_at IS NOT NULL,
                CASE WHEN s.card_id IS NULL THEN 0
                     WHEN s.phase = '{review}' THEN 1
                     WHEN s.phase IN ('{learning}', '{relearning}') THEN 2 END,
                s.due_at
         FROM {TAGGED_FROM} LEFT JOIN card_schedule s ON s.card_id = c.id
         WHERE {TAGGED_IN_SCOPE}",
        review = Phase::Review.as_str(),
        learning = Phase::Learning.as_str(),
        relearning = Phase::Relearning.as_str(),
    ))?;
    let rows = stmt.query_map([tag_id], |r| {
        let stage = match (r.get::<_, Option<i64>>(5)?, r.get::<_, Option<i64>>(6)?) {
            (Some(0), _) => Some(Stage::New),
            (Some(1), Some(due_at)) => Some(Stage::Review(due_at)),
            (Some(2), Some(due_at)) => Some(Stage::Step(due_at)),
            _ => None,
        };
        let Some(stage) = stage else {
            return Ok(None);
        };
        Ok(Some(Candidate {
            card_id: r.get(0)?,
            deck_id: r.get(1)?,
            seq: r.get(2)?,
            suspended: r.get(3)?,
            deleted: r.get(4)?,
            stage,
        }))
    })?;
    let mut candidates = Vec::new();
    for row in rows {
        candidates.extend(row?);
    }
    Ok(allocate(&allowances, candidates, now))
}

/// [`Store::next_study_change`] of tag `tag_id`, inside the caller's read
/// transaction: [`next_study_change_in`]'s rule over the live cards with
/// the tag that are not suspended, in the decks of a tag session.
fn tag_next_study_change_in(conn: &Connection, tag_id: &str, now: i64) -> Result<Option<i64>> {
    // Phases are our own constant strings, never user input.
    Ok(conn.query_row(
        &format!(
            "SELECT min(CASE WHEN s.phase = '{review}' THEN s.due_at
                             ELSE s.due_at - ?3 END)
             FROM {TAGGED_FROM} JOIN card_schedule s ON s.card_id = c.id
             WHERE {TAGGED_IN_SCOPE} AND c.deleted_at IS NULL AND c.suspended = 0
               AND (CASE WHEN s.phase = '{review}' THEN s.due_at
                         ELSE s.due_at - ?3 END) > ?2",
            review = Phase::Review.as_str()
        ),
        params![tag_id, now, EARLY_MS],
        |r| r.get(0),
    )?)
}

/// What [`Store::list_decks`] shows of deck `deck_id`: the lengths of its
/// [`study_input_in`] and the counts around them, from counts alone (no
/// card ids are read), inside the caller's read transaction.
pub(super) struct DeckCounts {
    /// `study_input_in(..).new.len()`.
    pub new: u32,
    /// `study_input_in(..).reviews.len()`.
    pub reviews: u32,
    /// `study_input_in(..).steps.len()`.
    pub steps: u32,
    /// Live cards, suspended ones included.
    pub live: u32,
    /// Live suspended cards.
    pub suspended: u32,
    /// Cards not offered now only because of the daily limits.
    pub held_back: u32,
    /// [`Store::next_study_change`].
    pub next_change_at: Option<i64>,
}

/// [`DeckCounts`] of live deck `deck_id`: today's counts, then one
/// aggregate pass over the deck's live cards with the same conditions as
/// [`study_input_in`] and [`next_study_change_in`]. The deck list test
/// `list_counts_equal_the_study_session_at_any_time` holds them equal.
pub(super) fn deck_counts_in(
    conn: &Connection,
    deck_id: &str,
    now: i64,
    day_start: i64,
) -> Result<DeckCounts> {
    let limits = settings_in(conn)?;
    let (reviews_today, new_today) = today_counts_in(conn, deck_id, day_start)?;
    // Phases are our own constant strings, never user input.
    let review = Phase::Review.as_str();
    let step_phases = format!(
        "'{}', '{}'",
        Phase::Learning.as_str(),
        Phase::Relearning.as_str()
    );
    let (live, suspended, new_total, due_reviews, steps, next): (
        i64,
        i64,
        i64,
        i64,
        i64,
        Option<i64>,
    ) = conn.query_row(
        &format!(
            "SELECT count(*),
                        coalesce(sum(c.suspended <> 0), 0),
                        coalesce(sum(c.suspended = 0 AND s.card_id IS NULL), 0),
                        coalesce(sum(c.suspended = 0 AND s.phase = '{review}'
                                     AND s.due_at <= ?2), 0),
                        coalesce(sum(c.suspended = 0 AND s.phase IN ({step_phases})
                                     AND s.due_at <= ?4), 0),
                        min(CASE WHEN c.suspended <> 0 THEN NULL
                                 WHEN s.phase = '{review}' AND s.due_at > ?2 THEN s.due_at
                                 WHEN s.phase <> '{review}' AND s.due_at - ?3 > ?2
                                     THEN s.due_at - ?3 END)
                 FROM cards c LEFT JOIN card_schedule s ON s.card_id = c.id
                 WHERE c.deck_id = ?1 AND c.deleted_at IS NULL"
        ),
        params![deck_id, now, EARLY_MS, now.saturating_add(EARLY_MS)],
        |r| {
            Ok((
                r.get(0)?,
                r.get(1)?,
                r.get(2)?,
                r.get(3)?,
                r.get(4)?,
                r.get(5)?,
            ))
        },
    )?;
    let count = |n: i64| u32::try_from(n.max(0)).unwrap_or(u32::MAX);
    let new_total = count(new_total);
    let due_reviews = count(due_reviews);
    let new = new_total.min(allowance(limits.max_new_per_day, new_today));
    let reviews = due_reviews.min(allowance(limits.max_reviews_per_day, reviews_today));
    Ok(DeckCounts {
        new,
        reviews,
        steps: count(steps),
        live: count(live),
        suspended: count(suspended),
        held_back: (new_total - new).saturating_add(due_reviews - reviews),
        next_change_at: next,
    })
}

#[cfg(test)]
mod tests;

#[cfg(test)]
mod tag_tests;
