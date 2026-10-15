//! Decks: create, rename, list with counts, soft delete and its undo
//! (ADR 0008, ADR 0012).

use rusqlite::{Connection, ErrorCode, OptionalExtension, TransactionBehavior, params};

use super::{Store, observe_max_id};
use crate::domain::{Deck, DeckSummary, deck_name_key};
use crate::error::{Error, Result};
use crate::time::now_ms;

impl Store {
    /// Creates a deck named `name` (trimmed).
    ///
    /// # Errors
    /// [`Error::EmptyDeckName`], [`Error::DuplicateDeckName`] if a live deck
    /// has that name, id and SQLite errors.
    pub fn create_deck(&mut self, name: &str) -> Result<Deck> {
        self.create_deck_at(name, now_ms())
    }

    /// [`Self::create_deck`] at time `now` (UTC milliseconds).
    pub(super) fn create_deck_at(&mut self, name: &str, now: i64) -> Result<Deck> {
        let name = deck_name(name)?;
        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        observe_max_id(&tx, &mut self.ids, "decks.id")?;
        let deck = Deck {
            id: self.ids.next(now)?,
            name: name.to_owned(),
            created_at: now,
            updated_at: now,
        };
        tx.execute(
            "INSERT INTO decks (id, name, name_key, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![
                deck.id,
                deck.name,
                deck_name_key(name),
                deck.created_at,
                deck.updated_at
            ],
        )
        .map_err(|e| duplicate_name(e, name))?;
        tx.commit()?;
        Ok(deck)
    }

    /// Renames deck `id` to `name` (trimmed). Renaming a deck to the name
    /// it already has changes nothing.
    ///
    /// # Errors
    /// [`Error::EmptyDeckName`], [`Error::DuplicateDeckName`] if another live
    /// deck has that name, [`Error::DeckNotFound`], SQLite errors.
    pub fn rename_deck(&mut self, id: &str, name: &str) -> Result<()> {
        self.rename_deck_at(id, name, now_ms())
    }

    /// [`Self::rename_deck`] at time `now` (UTC milliseconds).
    pub(super) fn rename_deck_at(&mut self, id: &str, name: &str, now: i64) -> Result<()> {
        let name = deck_name(name)?;
        let changed = self
            .conn
            .execute(
                "UPDATE decks SET name = ?2, name_key = ?3, updated_at = ?4
                 WHERE id = ?1 AND deleted_at IS NULL AND name IS NOT ?2",
                params![id, name, deck_name_key(name), now],
            )
            .map_err(|e| duplicate_name(e, name))?;
        if changed == 0 {
            // Either the name is unchanged, or there is no such deck.
            ensure_live_deck(&self.conn, id)?;
        }
        Ok(())
    }

    /// Every live deck with its number of new cards and of cards due at
    /// `now` (UTC milliseconds), sorted by name key ([`deck_name_key`]), then id. Daily
    /// limits are not applied.
    ///
    /// # Errors
    /// SQLite errors.
    pub fn list_decks(&self, now: i64) -> Result<Vec<DeckSummary>> {
        let mut stmt = self.conn.prepare(
            "SELECT d.id, d.name, d.created_at, d.updated_at,
                    count(c.id) FILTER (WHERE s.card_id IS NULL),
                    count(c.id) FILTER (WHERE s.due_at <= ?1)
             FROM decks d
             LEFT JOIN cards c ON c.deck_id = d.id AND c.deleted_at IS NULL
             LEFT JOIN card_schedule s ON s.card_id = c.id
             WHERE d.deleted_at IS NULL
             GROUP BY d.id",
        )?;
        let rows = stmt.query_map([now], |r| {
            Ok(DeckSummary {
                deck: Deck {
                    id: r.get(0)?,
                    name: r.get(1)?,
                    created_at: r.get(2)?,
                    updated_at: r.get(3)?,
                },
                new_count: r.get(4)?,
                due_count: r.get(5)?,
            })
        })?;
        let mut decks = rows.collect::<rusqlite::Result<Vec<_>>>()?;
        decks.sort_by_cached_key(|d| (deck_name_key(&d.deck.name), d.deck.id.clone()));
        Ok(decks)
    }

    /// Soft-deletes deck `id` and its live cards. They all get the same
    /// `deleted_at` stamp, greater than that of any card of the deck
    /// deleted earlier, so a later undo can restore exactly this batch.
    /// Their review history is kept.
    ///
    /// # Errors
    /// [`Error::DeckNotFound`], SQLite errors.
    pub fn delete_deck(&mut self, id: &str) -> Result<()> {
        self.delete_deck_at(id, now_ms())
    }

    /// [`Self::delete_deck`] at time `now` (UTC milliseconds).
    pub(super) fn delete_deck_at(&mut self, id: &str, now: i64) -> Result<()> {
        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        ensure_live_deck(&tx, id)?;
        let earlier: Option<i64> = tx.query_row(
            "SELECT max(deleted_at) FROM cards WHERE deck_id = ?1",
            [id],
            |r| r.get(0),
        )?;
        let stamp = earlier.map_or(now, |e| now.max(e.saturating_add(1)));
        tx.execute(
            "UPDATE decks SET deleted_at = ?2 WHERE id = ?1",
            params![id, stamp],
        )?;
        tx.execute(
            "UPDATE cards SET deleted_at = ?2 WHERE deck_id = ?1 AND deleted_at IS NULL",
            params![id, stamp],
        )?;
        tx.commit()?;
        Ok(())
    }

    /// Undoes [`Self::delete_deck`]: deck `id` and exactly the cards deleted
    /// with it (those sharing the deck's `deleted_at` stamp) become live
    /// again. Cards deleted on their own before the deck have an earlier
    /// stamp and stay deleted (ADR 0008, ADR 0012). Returns the deck.
    /// A deck that is already live is returned unchanged.
    ///
    /// # Errors
    /// [`Error::DeckNotFound`] if no deck has this id,
    /// [`Error::DuplicateDeckName`] if a live deck took its name meanwhile
    /// (nothing is changed), SQLite errors.
    pub fn undelete_deck(&mut self, id: &str) -> Result<Deck> {
        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let (deck, stamp): (Deck, Option<i64>) = tx
            .query_row(
                "SELECT id, name, created_at, updated_at, deleted_at FROM decks WHERE id = ?1",
                [id],
                |r| {
                    Ok((
                        Deck {
                            id: r.get(0)?,
                            name: r.get(1)?,
                            created_at: r.get(2)?,
                            updated_at: r.get(3)?,
                        },
                        r.get(4)?,
                    ))
                },
            )
            .optional()?
            .ok_or_else(|| Error::DeckNotFound(id.to_owned()))?;
        let Some(stamp) = stamp else {
            return Ok(deck);
        };
        tx.execute("UPDATE decks SET deleted_at = NULL WHERE id = ?1", [id])
            .map_err(|e| duplicate_name(e, &deck.name))?;
        tx.execute(
            "UPDATE cards SET deleted_at = NULL WHERE deck_id = ?1 AND deleted_at = ?2",
            params![id, stamp],
        )?;
        tx.commit()?;
        Ok(deck)
    }
}

/// `name` trimmed, if it is not empty.
fn deck_name(name: &str) -> Result<&str> {
    let name = name.trim();
    if name.is_empty() {
        Err(Error::EmptyDeckName)
    } else {
        Ok(name)
    }
}

/// Maps the live-name unique index's error to [`Error::DuplicateDeckName`].
/// `decks.id` is the only other unique key, and a new id is always above
/// the largest stored one, so a unique violation here is the name.
fn duplicate_name(err: rusqlite::Error, name: &str) -> Error {
    if let rusqlite::Error::SqliteFailure(e, _) = &err
        && e.code == ErrorCode::ConstraintViolation
        && e.extended_code == rusqlite::ffi::SQLITE_CONSTRAINT_UNIQUE
    {
        return Error::DuplicateDeckName(name.to_owned());
    }
    err.into()
}

/// Fails with [`Error::DeckNotFound`] unless deck `id` exists and is live.
pub(super) fn ensure_live_deck(conn: &Connection, id: &str) -> Result<()> {
    conn.query_row(
        "SELECT 1 FROM decks WHERE id = ?1 AND deleted_at IS NULL",
        [id],
        |_| Ok(()),
    )
    .optional()?
    .ok_or_else(|| Error::DeckNotFound(id.to_owned()))
}

#[cfg(test)]
mod tests;
