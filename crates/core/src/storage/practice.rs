//! What a practice goes over (ADR 0070): read only.

use rusqlite::{Connection, params};

use super::Store;
use super::decks::ensure_active_deck;
use super::review::{TAGGED_FROM, TAGGED_IN_SCOPE, ensure_tag};
use super::schedule::to_f32;
use crate::error::Result;
use crate::scheduling::StudyScope;
use crate::scheduling::practice::{PracticeCandidate, practice_order};
use crate::stats::retrievability;

impl Store {
    /// The cards a practice of `scope` shows at `now` (UTC milliseconds),
    /// in order: the live cards of the scope that are not suspended, in
    /// decks that are neither deleted nor archived, the reviewed ones by
    /// their retrievability now (lowest first; from the cached stability,
    /// difficulty and last review, as Card Information computes it), then
    /// the new ones oldest first, ties by id, at most
    /// `PRACTICE_LIMIT` (50). Every card is
    /// offered, due or not: practice ignores the schedule and the daily
    /// limits, since it records nothing.
    ///
    /// The read runs with `PRAGMA query_only` on (`query_only`), so a
    /// write in it would fail with `SQLITE_READONLY` instead of reaching
    /// the database.
    ///
    /// # Errors
    /// [`Error::DeckNotFound`](crate::Error::DeckNotFound),
    /// [`Error::DeckArchived`](crate::Error::DeckArchived),
    /// [`Error::TagNotFound`](crate::Error::TagNotFound), SQLite errors.
    pub fn practice_queue(&self, scope: &StudyScope, now: i64) -> Result<Vec<String>> {
        let candidates = query_only(&self.conn, |conn| {
            let tx = conn.unchecked_transaction()?;
            let candidates = match scope {
                StudyScope::Deck(deck_id) => {
                    ensure_active_deck(&tx, deck_id)?;
                    candidates_in(&tx, "cards c", "c.deck_id = ?1", deck_id, now)?
                }
                StudyScope::Tag(tag_id) => {
                    ensure_tag(&tx, tag_id)?;
                    candidates_in(&tx, TAGGED_FROM, TAGGED_IN_SCOPE, tag_id, now)?
                }
            };
            // Read only: ending it commits nothing.
            tx.finish()?;
            Ok(candidates)
        })?;
        Ok(practice_order(candidates))
    }

    /// Whether a practice of `scope` would show at least one card: the
    /// same cards as [`Self::practice_queue`], counted by one `EXISTS`
    /// without reading them, for enabling the entries. Run with
    /// `PRAGMA query_only` on, as `practice_queue`.
    ///
    /// # Errors
    /// As [`Self::practice_queue`].
    pub fn has_practisable(&self, scope: &StudyScope) -> Result<bool> {
        query_only(&self.conn, |conn| {
            let tx = conn.unchecked_transaction()?;
            let (from, condition, id) = match scope {
                StudyScope::Deck(deck_id) => {
                    ensure_active_deck(&tx, deck_id)?;
                    ("cards c", "c.deck_id = ?1", deck_id)
                }
                StudyScope::Tag(tag_id) => {
                    ensure_tag(&tx, tag_id)?;
                    (TAGGED_FROM, TAGGED_IN_SCOPE, tag_id)
                }
            };
            let any: bool = tx.query_row(
                &format!(
                    "SELECT EXISTS (SELECT 1 FROM {from}
                     WHERE {condition} AND {PRACTISABLE})"
                ),
                params![id],
                |r| r.get(0),
            )?;
            tx.finish()?;
            Ok(any)
        })
    }
}

/// The cards of a scope a practice shows (with the scope's own condition
/// on its decks): live and not suspended. Shared by the queue and
/// [`Store::has_practisable`], so they cannot disagree.
const PRACTISABLE: &str = "c.deleted_at IS NULL AND c.suspended = 0";

/// Runs `read` on `conn` with `PRAGMA query_only` on ([`QueryOnly`]):
/// while it is on, SQLite refuses any change to the database
/// (`SQLITE_READONLY`), so a write slipped into a practice's read fails
/// instead of landing (ADR 0070). The pragma is a setting of the
/// connection, not a write: it changes neither the file nor
/// `data_version`.
///
/// # Errors
/// A failure to set the pragma, or `read`'s error.
pub(super) fn query_only<T>(
    conn: &Connection,
    read: impl FnOnce(&Connection) -> Result<T>,
) -> Result<T> {
    let _guard = QueryOnly::new(conn)?;
    read(conn)
}

/// `PRAGMA query_only` on for as long as it lives: on in [`Self::new`],
/// back to what it was when dropped (off, unless a caller had it on), so a
/// read that returns early, fails or panics still leaves the connection
/// writable for the app's own writes, and a read nested in another one
/// never turns it off under the outer one (ADR 0073).
struct QueryOnly<'c> {
    conn: &'c Connection,
    was_on: bool,
}

impl<'c> QueryOnly<'c> {
    /// Turns `query_only` on for `conn`.
    ///
    /// # Errors
    /// If SQLite cannot read or set it; it is then as it was.
    fn new(conn: &'c Connection) -> Result<Self> {
        let was_on: bool = conn.query_row("PRAGMA query_only", [], |r| r.get(0))?;
        conn.pragma_update(None, "query_only", true)?;
        Ok(Self { conn, was_on })
    }
}

impl Drop for QueryOnly<'_> {
    fn drop(&mut self) {
        if self.was_on {
            return;
        }
        // Setting a pragma of an open connection does not fail in
        // practice; if it ever does, it is logged (no card content), and
        // the next write reports SQLITE_READONLY rather than writing.
        if let Err(err) = self.conn.pragma_update(None, "query_only", false) {
            tracing::warn!("cannot turn query_only off after a practice read: {err}");
        }
    }
}

/// The live, unsuspended cards of `from` (whose cards table is `c`) that
/// `scope` (a condition on `?1` = `id`) selects, with their retrievability
/// at `now`.
fn candidates_in(
    conn: &Connection,
    from: &str,
    scope: &str,
    id: &str,
    now: i64,
) -> Result<Vec<PracticeCandidate>> {
    let mut stmt = conn.prepare(&format!(
        "SELECT c.id, c.seq, s.stability, s.difficulty, s.last_review_at
         FROM {from} LEFT JOIN card_schedule s ON s.card_id = c.id
         WHERE {scope} AND {PRACTISABLE}"
    ))?;
    let rows =
        stmt.query_map(params![id], |r| {
            let memory: (Option<f64>, Option<f64>, Option<i64>) = (r.get(2)?, r.get(3)?, r.get(4)?);
            Ok(PracticeCandidate {
                card_id: r.get(0)?,
                seq: r.get(1)?,
                // A card without a schedule row is new: it has no memory.
                retrievability: match memory {
                    (Some(stability), Some(difficulty), Some(last_review_at)) => Some(
                        retrievability(to_f32(stability), to_f32(difficulty), last_review_at, now),
                    ),
                    _ => None,
                },
            })
        })?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

#[cfg(test)]
mod tests;
