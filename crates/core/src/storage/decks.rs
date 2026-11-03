//! Decks: create, rename, list with counts, soft delete and its undo
//! (ADR 0008, ADR 0012), archive and unarchive (ADR 0068).

use rusqlite::{Connection, ErrorCode, OptionalExtension, TransactionBehavior, params};

use super::review::deck_counts_in;
use super::{Store, observe_max_id};
use crate::domain::{
    ArchivedDeck, Deck, DeckSummary, MAX_DECK_NAME_BYTES, deck_name_fits, deck_name_key, has_nul,
};
use crate::error::{Error, Result};
use crate::time::now_ms;

impl Store {
    /// Creates a deck named `name` (trimmed).
    ///
    /// # Errors
    /// [`Error::EmptyDeckName`], [`Error::DeckNameTooLong`],
    /// [`Error::DuplicateDeckName`] if a live deck
    /// has that name ([`Error::ArchivedDeckName`] if that deck is
    /// archived), id and SQLite errors.
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
            archived_at: None,
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
        .map_err(|e| name_taken(&tx, e, name))?;
        tx.commit()?;
        Ok(deck)
    }

    /// Renames deck `id` to `name` (trimmed). Renaming a deck to the name
    /// it already has changes nothing.
    ///
    /// # Errors
    /// [`Error::EmptyDeckName`], [`Error::DeckNameTooLong`],
    /// [`Error::DuplicateDeckName`] if another live
    /// deck has that name ([`Error::ArchivedDeckName`] if that deck is
    /// archived), [`Error::DeckNotFound`], SQLite errors. An archived deck
    /// can be renamed in core; the app offers no rename for one.
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
            .map_err(|e| name_taken(&self.conn, e, name))?;
        if changed == 0 {
            // Either the name is unchanged, or there is no such deck.
            ensure_live_deck(&self.conn, id)?;
        }
        Ok(())
    }

    /// Every live deck that is not archived with what it has to study at
    /// `now` (UTC milliseconds), sorted by name key ([`deck_name_key`]), then id. The
    /// counts are the lengths of [`Store::study_input`] for the study day
    /// that began at `day_start`: due reviews and step cards (those due
    /// within 20 minutes included), and the new cards still allowed today.
    /// Each summary also has the deck's card and suspended counts, the
    /// cards the daily limits hold back, and the next study change
    /// ([`Store::next_study_change`]), read in the same transaction.
    ///
    /// # Errors
    /// SQLite errors.
    pub fn list_decks(&self, now: i64, day_start: i64) -> Result<Vec<DeckSummary>> {
        let tx = self.conn.unchecked_transaction()?;
        let decks = live_decks(&tx, Archived::Without)?;
        let mut summaries = Vec::with_capacity(decks.len());
        for deck in decks {
            // The deck page's own rules (ADR 0014), so both always agree;
            // counted without reading the cards' ids (ADR 0041).
            let c = deck_counts_in(&tx, &deck.id, now, day_start)?;
            summaries.push(DeckSummary {
                deck,
                new_count: c.new,
                due_count: c.reviews.saturating_add(c.steps),
                card_count: c.live,
                suspended_count: c.suspended,
                held_back_count: c.held_back,
                next_change_at: c.next_change_at,
            });
        }
        // Read only: ending it commits nothing.
        tx.finish()?;
        Ok(summaries)
    }

    /// Every live deck that is not archived, sorted as in
    /// [`Self::list_decks`], without the counts: one query, whatever the
    /// number of cards. The decks cards can be moved or merged into.
    ///
    /// # Errors
    /// SQLite errors.
    pub fn decks(&self) -> Result<Vec<Deck>> {
        live_decks(&self.conn, Archived::Without)
    }

    /// Every live deck, archived ones included (each says whether it is),
    /// sorted as in [`Self::list_decks`]: for what names any live deck
    /// (the name check, All Cards' deck filter, import conflicts).
    ///
    /// # Errors
    /// SQLite errors.
    pub fn decks_with_archived(&self) -> Result<Vec<Deck>> {
        live_decks(&self.conn, Archived::With)
    }

    /// The archived decks with their card counts, sorted as in
    /// [`Self::list_decks`], for the Archived Decks page (ADR 0068). One
    /// query.
    ///
    /// # Errors
    /// SQLite errors.
    pub fn archived_decks(&self) -> Result<Vec<ArchivedDeck>> {
        let mut stmt = self.conn.prepare(&format!(
            "SELECT {DECK_COLUMNS},
                    (SELECT count(*) FROM cards c
                     WHERE c.deck_id = decks.id AND c.deleted_at IS NULL)
             FROM decks WHERE deleted_at IS NULL AND archived_at IS NOT NULL"
        ))?;
        let mut decks = stmt
            .query_map([], |r| {
                Ok(ArchivedDeck {
                    deck: deck_from_row(r)?,
                    card_count: u32::try_from(r.get::<_, i64>(5)?.max(0)).unwrap_or(u32::MAX),
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        decks.sort_by_cached_key(|d| (deck_name_key(&d.deck.name), d.deck.id.clone()));
        Ok(decks)
    }

    /// Whether any live card is in a live deck, archived ones included:
    /// one indexed lookup, for All Cards' empty state (ADR 0068).
    ///
    /// # Errors
    /// SQLite errors.
    pub fn has_live_cards(&self) -> Result<bool> {
        Ok(self.conn.query_row(
            "SELECT EXISTS (SELECT 1 FROM cards c JOIN decks d ON d.id = c.deck_id
                            WHERE c.deleted_at IS NULL AND d.deleted_at IS NULL)",
            [],
            |r| r.get(0),
        )?)
    }

    /// Live deck `id`, archived or not (its `archived_at` says), without
    /// its counts.
    ///
    /// # Errors
    /// [`Error::DeckNotFound`] if there is no such deck or it was deleted,
    /// SQLite errors.
    pub fn deck(&self, id: &str) -> Result<Deck> {
        live_deck(&self.conn, id)
    }

    /// Archives deck `id` at the current time: it leaves the deck list,
    /// study and the global forecast and retention, and keeps its name,
    /// its cards and their history (ADR 0068). Nothing else of the deck
    /// changes, `updated_at` included, so an export made before still
    /// imports as already present. Returns the deck, archived.
    ///
    /// # Errors
    /// [`Error::DeckNotFound`] if there is no such deck or it was deleted,
    /// [`Error::DeckArchived`] if it is archived already (its stamp is
    /// kept, so an Undo never unarchives a deck archived earlier), SQLite
    /// errors. On error nothing is written.
    pub fn archive_deck(&mut self, id: &str) -> Result<Deck> {
        self.archive_deck_at(id, now_ms())
    }

    /// [`Self::archive_deck`] at time `now` (UTC milliseconds).
    pub(super) fn archive_deck_at(&mut self, id: &str, now: i64) -> Result<Deck> {
        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let mut deck = live_deck(&tx, id)?;
        if deck.is_archived() {
            return Err(Error::DeckArchived(id.to_owned()));
        }
        tx.execute(
            "UPDATE decks SET archived_at = ?2 WHERE id = ?1",
            params![id, now],
        )?;
        tx.commit()?;
        deck.archived_at = Some(now);
        Ok(deck)
    }

    /// Unarchives deck `id`: it is back in the deck list and in study, as
    /// before it was archived. The Undo of [`Self::archive_deck`]. A deck
    /// that is not archived is returned unchanged. Returns the deck.
    ///
    /// # Errors
    /// [`Error::DeckNotFound`] if there is no such deck or it was deleted,
    /// SQLite errors.
    pub fn unarchive_deck(&mut self, id: &str) -> Result<Deck> {
        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let mut deck = live_deck(&tx, id)?;
        if deck.is_archived() {
            tx.execute("UPDATE decks SET archived_at = NULL WHERE id = ?1", [id])?;
            tx.commit()?;
            deck.archived_at = None;
        }
        Ok(deck)
    }

    /// How many live cards deck `id` has, suspended ones included: what
    /// Merge Into… says it moves (ADR 0066). One count on the deck's index.
    ///
    /// # Errors
    /// [`Error::DeckNotFound`] if there is no such deck or it was deleted,
    /// SQLite errors.
    pub fn card_count(&self, id: &str) -> Result<usize> {
        let tx = self.conn.unchecked_transaction()?;
        ensure_live_deck(&tx, id)?;
        let count: i64 = tx.query_row(
            "SELECT count(*) FROM cards WHERE deck_id = ?1 AND deleted_at IS NULL",
            [id],
            |r| r.get(0),
        )?;
        // Read only: ending it commits nothing.
        tx.finish()?;
        Ok(usize::try_from(count).unwrap_or(0))
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
        let stamp = deletion_stamp(&tx, id, now)?;
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
    /// ([`Error::ArchivedDeckName`] if that deck is archived; nothing is
    /// changed), SQLite errors. A deck archived when it was deleted comes
    /// back archived.
    pub fn undelete_deck(&mut self, id: &str) -> Result<Deck> {
        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let (deck, stamp): (Deck, Option<i64>) = tx
            .query_row(
                &format!("SELECT {DECK_COLUMNS}, deleted_at FROM decks WHERE id = ?1"),
                [id],
                |r| Ok((deck_from_row(r)?, r.get(5)?)),
            )
            .optional()?
            .ok_or_else(|| Error::DeckNotFound(id.to_owned()))?;
        let Some(stamp) = stamp else {
            return Ok(deck);
        };
        tx.execute("UPDATE decks SET deleted_at = NULL WHERE id = ?1", [id])
            .map_err(|e| name_taken(&tx, e, &deck.name))?;
        tx.execute(
            "UPDATE cards SET deleted_at = NULL WHERE deck_id = ?1 AND deleted_at = ?2",
            params![id, stamp],
        )?;
        tx.commit()?;
        Ok(deck)
    }
}

/// The columns [`deck_from_row`] reads, in its order.
pub(super) const DECK_COLUMNS: &str = "id, name, created_at, updated_at, archived_at";

/// Whether [`live_decks`] lists the archived decks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Archived {
    With,
    Without,
}

/// Every live deck (archived ones only `With`), sorted by name key
/// ([`deck_name_key`]), then id.
fn live_decks(conn: &Connection, archived: Archived) -> Result<Vec<Deck>> {
    let filter = match archived {
        Archived::With => "",
        Archived::Without => " AND archived_at IS NULL",
    };
    let mut stmt = conn.prepare(&format!(
        "SELECT {DECK_COLUMNS} FROM decks WHERE deleted_at IS NULL{filter}"
    ))?;
    let mut decks = stmt
        .query_map([], deck_from_row)?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    decks.sort_by_cached_key(|d| (deck_name_key(&d.name), d.id.clone()));
    Ok(decks)
}

/// Live deck `id`, archived or not.
fn live_deck(conn: &Connection, id: &str) -> Result<Deck> {
    conn.query_row(
        &format!("SELECT {DECK_COLUMNS} FROM decks WHERE id = ?1 AND deleted_at IS NULL"),
        [id],
        deck_from_row,
    )
    .optional()?
    .ok_or_else(|| Error::DeckNotFound(id.to_owned()))
}

/// A deck from a row whose first columns are [`DECK_COLUMNS`].
pub(super) fn deck_from_row(r: &rusqlite::Row<'_>) -> rusqlite::Result<Deck> {
    Ok(Deck {
        id: r.get(0)?,
        name: r.get(1)?,
        created_at: r.get(2)?,
        updated_at: r.get(3)?,
        archived_at: r.get(4)?,
    })
}

/// `name` trimmed, if it is not empty and not too long to import again.
fn deck_name(name: &str) -> Result<&str> {
    let name = name.trim();
    if name.is_empty() {
        Err(Error::EmptyDeckName)
    } else if has_nul(name) {
        Err(Error::ContainsNul)
    } else if !deck_name_fits(name) {
        Err(Error::DeckNameTooLong {
            limit: MAX_DECK_NAME_BYTES,
        })
    } else {
        Ok(name)
    }
}

/// Maps the live-name unique index's error to [`Error::DuplicateDeckName`],
/// or to [`Error::ArchivedDeckName`] when the live deck that has the name
/// (read on `conn`, inside the failed write's transaction) is archived.
/// `decks.id` is the only other unique key, and a new id is always above
/// the largest stored one, so a unique violation here is the name.
pub(super) fn name_taken(conn: &Connection, err: rusqlite::Error, name: &str) -> Error {
    if let rusqlite::Error::SqliteFailure(e, _) = &err
        && e.code == ErrorCode::ConstraintViolation
        && e.extended_code == rusqlite::ffi::SQLITE_CONSTRAINT_UNIQUE
    {
        let archived = conn
            .query_row(
                "SELECT archived_at IS NOT NULL FROM decks
                 WHERE name_key = ?1 AND deleted_at IS NULL",
                [deck_name_key(name.trim())],
                |r| r.get::<_, bool>(0),
            )
            .optional();
        return match archived {
            Ok(Some(true)) => Error::ArchivedDeckName(name.to_owned()),
            // Not found again, or not readable: the plain duplicate.
            Ok(_) | Err(_) => Error::DuplicateDeckName(name.to_owned()),
        };
    }
    err.into()
}

/// The `deleted_at` stamp for deleting deck `id` at `now`: `now`, or one
/// past the latest stamp of a card of the deck deleted earlier if that is
/// later (a clock that went backwards, or the same millisecond), so an
/// undo never takes such a card for one of the deck's batch (ADR 0012).
/// Shared by [`Store::delete_deck`] and [`Store::merge_deck`] (ADR 0066).
pub(super) fn deletion_stamp(conn: &Connection, id: &str, now: i64) -> Result<i64> {
    let earlier: Option<i64> = conn.query_row(
        "SELECT max(deleted_at) FROM cards WHERE deck_id = ?1",
        [id],
        |r| r.get(0),
    )?;
    Ok(earlier.map_or(now, |e| now.max(e.saturating_add(1))))
}

/// Fails with [`Error::DeckNotFound`] unless deck `id` exists and is live
/// (archived or not).
pub(super) fn ensure_live_deck(conn: &Connection, id: &str) -> Result<()> {
    conn.query_row(
        "SELECT 1 FROM decks WHERE id = ?1 AND deleted_at IS NULL",
        [id],
        |_| Ok(()),
    )
    .optional()?
    .ok_or_else(|| Error::DeckNotFound(id.to_owned()))
}

/// Fails with [`Error::DeckNotFound`] unless deck `id` exists and is live,
/// and with [`Error::DeckArchived`] if it is archived: for study and for a
/// merge's target (ADR 0068).
pub(super) fn ensure_active_deck(conn: &Connection, id: &str) -> Result<()> {
    let archived: bool = conn
        .query_row(
            "SELECT archived_at IS NOT NULL FROM decks WHERE id = ?1 AND deleted_at IS NULL",
            [id],
            |r| r.get(0),
        )
        .optional()?
        .ok_or_else(|| Error::DeckNotFound(id.to_owned()))?;
    if archived {
        return Err(Error::DeckArchived(id.to_owned()));
    }
    Ok(())
}

#[cfg(test)]
mod tests;

#[cfg(test)]
mod archive_tests;
