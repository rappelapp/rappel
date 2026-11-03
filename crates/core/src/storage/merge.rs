//! Merge a deck into another, and its undo (SPEC §4.6, ADR 0066).
//!
//! A merge moves every card of the source deck, live and deleted, into the
//! target deck and soft-deletes the emptied source, in one transaction. A
//! card keeps everything but its deck: content, `updated_at`, tags,
//! suspension, `deleted_at`, its log and so its schedule. The source's
//! stamp is made as Delete Deck makes it ([`deletion_stamp`]), and the
//! target's later deletes stamp past every card it holds, so the undo rules
//! of Delete Deck (ADR 0012) and the import's revive rules (SPEC §4.1) stay
//! as they were: a card deleted before the merge is never revived with
//! either deck.

use rusqlite::{OptionalExtension, TransactionBehavior, params};

use super::Store;
use super::decks::{
    DECK_COLUMNS, deck_from_row, deletion_stamp, ensure_active_deck, ensure_live_deck, name_taken,
};
use crate::domain::Deck;
use crate::error::{Error, Result};
use crate::time::now_ms;

/// What [`Store::merge_deck`] did, for its undo ([`Store::unmerge_deck`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Merged {
    /// The deck merged, now deleted.
    pub(crate) source: String,
    /// The deck its cards went to.
    pub(crate) target: String,
    /// The source's `deleted_at`.
    pub(crate) stamp: i64,
    /// Every card moved, live and deleted, oldest first.
    pub(crate) cards: Vec<String>,
    /// How many of them were live.
    pub(crate) live: usize,
}

impl Store {
    /// Merges deck `source` into deck `target`: every card of `source`,
    /// deleted ones included, moves to `target` with its history, and
    /// `source` is soft-deleted, all in one transaction (ADR 0066). The
    /// cards' `updated_at` is not touched, as in [`Self::move_cards`].
    ///
    /// # Errors
    /// [`Error::MergeIntoItself`] if both are the same deck,
    /// [`Error::DeckNotFound`] if either is not a live deck,
    /// [`Error::DeckArchived`] if `target` is archived (ADR 0068: its cards
    /// would leave study unseen), SQLite errors. An archived `source` is
    /// merged like any deck; the app does not offer it. On error nothing is
    /// written.
    pub fn merge_deck(&mut self, source: &str, target: &str) -> Result<Merged> {
        self.merge_deck_at(source, target, now_ms())
    }

    /// [`Self::merge_deck`] at time `now` (UTC milliseconds).
    pub(super) fn merge_deck_at(&mut self, source: &str, target: &str, now: i64) -> Result<Merged> {
        if source == target {
            return Err(Error::MergeIntoItself(source.to_owned()));
        }
        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        ensure_live_deck(&tx, source)?;
        ensure_active_deck(&tx, target)?;
        let mut cards = Vec::new();
        let mut live = 0;
        {
            let mut stmt = tx.prepare(
                "SELECT id, deleted_at IS NULL FROM cards WHERE deck_id = ?1 ORDER BY seq",
            )?;
            let mut rows = stmt.query([source])?;
            while let Some(row) = rows.next()? {
                cards.push(row.get::<_, String>(0)?);
                if row.get::<_, bool>(1)? {
                    live += 1;
                }
            }
        }
        // Stamped before the cards leave, as Delete Deck would stamp it.
        let stamp = deletion_stamp(&tx, source, now)?;
        // One statement for every card, on the deck's index (ADR 0066).
        tx.execute(
            "UPDATE cards SET deck_id = ?2 WHERE deck_id = ?1",
            params![source, target],
        )?;
        tx.execute(
            "UPDATE decks SET deleted_at = ?2 WHERE id = ?1",
            params![source, stamp],
        )?;
        tx.commit()?;
        Ok(Merged {
            source: source.to_owned(),
            target: target.to_owned(),
            stamp,
            cards,
            live,
        })
    }

    /// Undoes [`Self::merge_deck`] in one transaction: the source deck is
    /// live again, and every card of `merged` that is still in the target
    /// goes back to it, deleted or not, edited or studied since or not. A
    /// card moved to another deck since stays there, as with
    /// [`Self::unmove_cards`]. A source already live again (revived by an
    /// import) stays live and gets its cards back. Returns the source deck.
    ///
    /// # Errors
    /// [`Error::DeckNotFound`] if the source no longer exists or was
    /// deleted again since (another stamp), or the target is not live (its
    /// delete is undone first); [`Error::DuplicateDeckName`] if a live deck
    /// took the source's name meanwhile; SQLite errors. On error nothing
    /// is written.
    pub fn unmerge_deck(&mut self, merged: &Merged) -> Result<Deck> {
        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let (deck, deleted_at): (Deck, Option<i64>) = tx
            .query_row(
                &format!("SELECT {DECK_COLUMNS}, deleted_at FROM decks WHERE id = ?1"),
                [&merged.source],
                |r| Ok((deck_from_row(r)?, r.get(5)?)),
            )
            .optional()?
            .ok_or_else(|| Error::DeckNotFound(merged.source.clone()))?;
        match deleted_at {
            None => {}
            Some(stamp) if stamp == merged.stamp => {
                tx.execute(
                    "UPDATE decks SET deleted_at = NULL WHERE id = ?1",
                    [&merged.source],
                )
                .map_err(|e| name_taken(&tx, e, &deck.name))?;
            }
            Some(_) => return Err(Error::DeckNotFound(merged.source.clone())),
        }
        ensure_live_deck(&tx, &merged.target)?;
        // One statement for every card: the ids as a JSON array, read by
        // SQLite's `json_each` (built in since SQLite 3.38). Strings are
        // escaped by `serde_json`, and the ids are bound, never spliced in.
        let ids = serde_json::Value::from(merged.cards.clone()).to_string();
        tx.execute(
            "UPDATE cards SET deck_id = ?1
             WHERE deck_id = ?2 AND id IN (SELECT value FROM json_each(?3))",
            params![merged.source, merged.target, ids],
        )?;
        tx.commit()?;
        Ok(deck)
    }
}

#[cfg(test)]
mod tests;
