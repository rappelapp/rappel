//! Cards: add, edit, list, soft delete (ADR 0008) and its undo (ADR 0013).

use rusqlite::{Connection, OptionalExtension, Row, TransactionBehavior, params};

use super::decks::ensure_live_deck;
use super::{Store, observe_max_id};
use crate::domain::{Card, CardSide, MAX_CARD_SIDE_BYTES, card_side_fits, has_nul};
use crate::error::{Error, Result};
use crate::time::now_ms;

/// The columns [`card_from_row`] reads, for a query with `cards` as `c`.
pub(super) const CARD_COLUMNS: &str =
    "c.id, c.deck_id, c.front_md, c.back_md, c.created_at, c.updated_at";

/// A [`Card`] from a row selected with [`CARD_COLUMNS`].
pub(super) fn card_from_row(r: &Row<'_>) -> rusqlite::Result<Card> {
    Ok(Card {
        id: r.get(0)?,
        deck_id: r.get(1)?,
        front_md: r.get(2)?,
        back_md: r.get(3)?,
        created_at: r.get(4)?,
        updated_at: r.get(5)?,
    })
}

impl Store {
    /// Adds a card to deck `deck_id`. The Markdown is stored as given.
    ///
    /// # Errors
    /// [`Error::EmptyCardFront`] if the front is blank,
    /// [`Error::CardSideTooLong`],
    /// [`Error::DeckNotFound`], id and SQLite errors.
    pub fn add_card(&mut self, deck_id: &str, front_md: &str, back_md: &str) -> Result<Card> {
        self.add_card_at(deck_id, front_md, back_md, now_ms())
    }

    /// [`Self::add_card`] at time `now` (UTC milliseconds).
    pub(super) fn add_card_at(
        &mut self,
        deck_id: &str,
        front_md: &str,
        back_md: &str,
        now: i64,
    ) -> Result<Card> {
        check_sides(front_md, back_md)?;
        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        ensure_live_deck(&tx, deck_id)?;
        observe_max_id(&tx, &mut self.ids, "cards.id")?;
        let card = Card {
            id: self.ids.next(now)?,
            deck_id: deck_id.to_owned(),
            front_md: front_md.to_owned(),
            back_md: back_md.to_owned(),
            created_at: now,
            updated_at: now,
        };
        tx.execute(
            "INSERT INTO cards (id, deck_id, front_md, back_md, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                card.id,
                card.deck_id,
                card.front_md,
                card.back_md,
                card.created_at,
                card.updated_at
            ],
        )?;
        tx.commit()?;
        Ok(card)
    }

    /// Replaces the front and back of card `id` and returns the card as
    /// stored.
    ///
    /// # Errors
    /// [`Error::EmptyCardFront`], [`Error::CardSideTooLong`],
    /// [`Error::CardNotFound`], SQLite errors.
    pub fn edit_card(&mut self, id: &str, front_md: &str, back_md: &str) -> Result<Card> {
        self.edit_card_at(id, front_md, back_md, now_ms())
    }

    /// [`Self::edit_card`] at time `now` (UTC milliseconds).
    pub(super) fn edit_card_at(
        &mut self,
        id: &str,
        front_md: &str,
        back_md: &str,
        now: i64,
    ) -> Result<Card> {
        check_sides(front_md, back_md)?;
        // One statement, so the returned card is exactly what was written.
        self.conn
            .query_row(
                // RETURNING takes no table-qualified names; same order as
                // CARD_COLUMNS.
                "UPDATE cards SET front_md = ?2, back_md = ?3, updated_at = ?4
                 WHERE id = ?1 AND deleted_at IS NULL
                 RETURNING id, deck_id, front_md, back_md, created_at, updated_at",
                params![id, front_md, back_md, now],
                card_from_row,
            )
            .optional()?
            .ok_or_else(|| Error::CardNotFound(id.to_owned()))
    }

    /// Card `id`.
    ///
    /// # Errors
    /// [`Error::CardNotFound`], SQLite errors.
    pub fn card(&self, id: &str) -> Result<Card> {
        live_card(&self.conn, id)
    }

    /// The live cards of deck `deck_id`, oldest first.
    ///
    /// # Errors
    /// [`Error::DeckNotFound`], SQLite errors.
    pub fn list_cards(&self, deck_id: &str) -> Result<Vec<Card>> {
        ensure_live_deck(&self.conn, deck_id)?;
        let mut stmt = self.conn.prepare(&format!(
            "SELECT {CARD_COLUMNS} FROM cards c
             WHERE c.deck_id = ?1 AND c.deleted_at IS NULL ORDER BY c.seq"
        ))?;
        let rows = stmt.query_map([deck_id], card_from_row)?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    /// Soft-deletes card `id`. Its review history is kept.
    ///
    /// # Errors
    /// [`Error::CardNotFound`], SQLite errors.
    pub fn delete_card(&mut self, id: &str) -> Result<()> {
        self.delete_card_at(id, now_ms())
    }

    /// [`Self::delete_card`] at time `now` (UTC milliseconds).
    pub(super) fn delete_card_at(&mut self, id: &str, now: i64) -> Result<()> {
        let changed = self.conn.execute(
            "UPDATE cards SET deleted_at = ?2 WHERE id = ?1 AND deleted_at IS NULL",
            params![id, now],
        )?;
        if changed == 0 {
            return Err(Error::CardNotFound(id.to_owned()));
        }
        Ok(())
    }

    /// Undoes [`Self::delete_card`]: card `id` is live again, with its
    /// content, review history and place in creation order. A live card is
    /// returned unchanged, so a second Undo is harmless. `updated_at` is
    /// not touched, as delete did not touch it.
    ///
    /// A card never comes back into a deleted deck: that fails with
    /// [`Error::DeckNotFound`] and changes nothing (ADR 0013).
    ///
    /// # Errors
    /// [`Error::CardNotFound`] for an unknown id, [`Error::DeckNotFound`],
    /// SQLite errors.
    pub fn undelete_card(&mut self, id: &str) -> Result<Card> {
        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let (card, deleted_at): (Card, Option<i64>) = tx
            .query_row(
                &format!("SELECT {CARD_COLUMNS}, c.deleted_at FROM cards c WHERE c.id = ?1"),
                [id],
                |r| Ok((card_from_row(r)?, r.get(6)?)),
            )
            .optional()?
            .ok_or_else(|| Error::CardNotFound(id.to_owned()))?;
        if deleted_at.is_none() {
            return Ok(card);
        }
        ensure_live_deck(&tx, &card.deck_id)?;
        tx.execute("UPDATE cards SET deleted_at = NULL WHERE id = ?1", [id])?;
        tx.commit()?;
        Ok(card)
    }
}

/// Card `id` if it exists and is live, else [`Error::CardNotFound`].
pub(super) fn live_card(conn: &Connection, id: &str) -> Result<Card> {
    conn.query_row(
        &format!("SELECT {CARD_COLUMNS} FROM cards c WHERE c.id = ?1 AND c.deleted_at IS NULL"),
        [id],
        card_from_row,
    )
    .optional()?
    .ok_or_else(|| Error::CardNotFound(id.to_owned()))
}

/// A blank front, a side too long to import again, or a NUL is refused.
fn check_sides(front_md: &str, back_md: &str) -> Result<()> {
    if front_md.trim().is_empty() {
        return Err(Error::EmptyCardFront);
    }
    for (side, text) in [(CardSide::Front, front_md), (CardSide::Back, back_md)] {
        if has_nul(text) {
            return Err(Error::ContainsNul);
        }
        if !card_side_fits(text) {
            return Err(Error::CardSideTooLong {
                side,
                limit: MAX_CARD_SIDE_BYTES,
            });
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests;
