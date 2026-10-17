//! Tags on cards: flat, unique by name key, never deleted (ADR 0029).

use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};

use super::cards::live_card;
use super::{Store, observe_max_id};
use crate::domain::{Tag, check_tag_name, name_key};
use crate::error::{Error, Result};
use crate::time::now_ms;

impl Store {
    /// Every tag, unused ones included, ordered by name key.
    ///
    /// # Errors
    /// SQLite errors.
    pub fn tags(&self) -> Result<Vec<Tag>> {
        let mut stmt = self
            .conn
            .prepare("SELECT id, name FROM tags ORDER BY name_key, id")?;
        let rows = stmt.query_map([], tag_from_row)?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    /// The tags of live card `card_id`, ordered by name key.
    ///
    /// # Errors
    /// [`Error::CardNotFound`] for a missing or deleted card, SQLite errors.
    pub fn card_tags(&self, card_id: &str) -> Result<Vec<Tag>> {
        live_card(&self.conn, card_id)?;
        let mut stmt = self.conn.prepare(
            "SELECT t.id, t.name FROM card_tags ct JOIN tags t ON t.id = ct.tag_id
             WHERE ct.card_id = ?1 ORDER BY t.name_key, t.id",
        )?;
        let rows = stmt.query_map([card_id], tag_from_row)?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    /// Puts the tag named `name` (trimmed) on live card `card_id`, and
    /// returns it. The tag whose name key is `name`'s is used, with its own
    /// spelling, if there is one; else a new tag is created. A tag the
    /// card already has is left as it is. The card itself (and its
    /// `updated_at`) is not changed.
    ///
    /// # Errors
    /// The errors of [`check_tag_name`](crate::domain::check_tag_name),
    /// [`Error::CardNotFound`] for a missing or deleted card, id and SQLite
    /// errors. On error nothing is written.
    pub fn add_tag(&mut self, card_id: &str, name: &str) -> Result<Tag> {
        self.add_tag_at(card_id, name, now_ms())
    }

    /// [`Self::add_tag`] at time `now` (UTC milliseconds, for a new id).
    pub(super) fn add_tag_at(&mut self, card_id: &str, name: &str, now: i64) -> Result<Tag> {
        let name = check_tag_name(name)?;
        let key = name_key(name);
        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        live_card(&tx, card_id)?;
        let tag = if let Some(tag) = tag_by_key(&tx, &key)? {
            tag
        } else {
            observe_max_id(&tx, &mut self.ids, "tags.id")?;
            let tag = Tag {
                id: self.ids.next(now)?,
                name: name.to_owned(),
            };
            tx.execute(
                "INSERT INTO tags (id, name, name_key) VALUES (?1, ?2, ?3)",
                params![tag.id, tag.name, key],
            )?;
            tag
        };
        tx.execute(
            "INSERT INTO card_tags (card_id, tag_id) VALUES (?1, ?2)
             ON CONFLICT DO NOTHING",
            params![card_id, tag.id],
        )?;
        tx.commit()?;
        Ok(tag)
    }

    /// Takes tag `tag_id` off live card `card_id`. Returns whether the card
    /// had it. The tag itself stays, even if no card has it any more.
    ///
    /// # Errors
    /// [`Error::CardNotFound`] for a missing or deleted card,
    /// [`Error::TagNotFound`], SQLite errors.
    pub fn remove_tag(&mut self, card_id: &str, tag_id: &str) -> Result<bool> {
        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        live_card(&tx, card_id)?;
        let exists = tx
            .query_row("SELECT 1 FROM tags WHERE id = ?1", [tag_id], |_| Ok(()))
            .optional()?
            .is_some();
        if !exists {
            return Err(Error::TagNotFound(tag_id.to_owned()));
        }
        let removed = tx.execute(
            "DELETE FROM card_tags WHERE card_id = ?1 AND tag_id = ?2",
            params![card_id, tag_id],
        )?;
        tx.commit()?;
        Ok(removed > 0)
    }
}

/// The tag whose name key is `key`.
fn tag_by_key(conn: &Connection, key: &str) -> Result<Option<Tag>> {
    Ok(conn
        .query_row(
            "SELECT id, name FROM tags WHERE name_key = ?1",
            [key],
            tag_from_row,
        )
        .optional()?)
}

fn tag_from_row(r: &rusqlite::Row<'_>) -> rusqlite::Result<Tag> {
    Ok(Tag {
        id: r.get(0)?,
        name: r.get(1)?,
    })
}

#[cfg(test)]
mod tests;
