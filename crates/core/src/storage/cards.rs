//! Cards: add, edit, list, soft delete (ADR 0008).

use rusqlite::{Connection, OptionalExtension, Row, TransactionBehavior, params};

use super::decks::ensure_live_deck;
use super::{Store, observe_max_id};
use crate::domain::Card;
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
        check_front(front_md)?;
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

    /// Replaces the front and back of card `id`.
    ///
    /// # Errors
    /// [`Error::EmptyCardFront`], [`Error::CardNotFound`], SQLite errors.
    pub fn edit_card(&mut self, id: &str, front_md: &str, back_md: &str) -> Result<()> {
        self.edit_card_at(id, front_md, back_md, now_ms())
    }

    /// [`Self::edit_card`] at time `now` (UTC milliseconds).
    pub(super) fn edit_card_at(
        &mut self,
        id: &str,
        front_md: &str,
        back_md: &str,
        now: i64,
    ) -> Result<()> {
        check_front(front_md)?;
        let changed = self.conn.execute(
            "UPDATE cards SET front_md = ?2, back_md = ?3, updated_at = ?4
             WHERE id = ?1 AND deleted_at IS NULL",
            params![id, front_md, back_md, now],
        )?;
        if changed == 0 {
            return Err(Error::CardNotFound(id.to_owned()));
        }
        Ok(())
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

fn check_front(front_md: &str) -> Result<()> {
    if front_md.trim().is_empty() {
        Err(Error::EmptyCardFront)
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests;
