//! Finding the other cards with the same front, for the card editor's
//! note (#119, ADR 0072). No index of its own: the search index already
//! holds each front's folded plain text (ADR 0035), so one FTS5 phrase
//! query narrows the cards down and Rust keeps the whole-front matches.

use rusqlite::Connection;

use super::Store;
use super::search::starts_token;
use crate::content::search_text;
use crate::error::Result;

/// Another live card whose front reads the same (`front_key`: case, accents and spacing ignored).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SameFront {
    /// The card.
    pub card_id: String,
    /// Its deck.
    pub deck_id: String,
    /// The name of its deck.
    pub deck_name: String,
    /// Its deck is archived (ADR 0068).
    pub archived: bool,
}

/// What two fronts are compared by: the plain text the user reads
/// (Markdown syntax left out, math and code as their source, an image as
/// its alt text, as the search index has it), in lower case without
/// accents, every run of whitespace (newlines included) as one space and
/// none at the ends. Punctuation stays: "Hola!" and "Hola" differ.
#[must_use]
pub(crate) fn front_key(front_md: &str) -> String {
    collapse_whitespace(&search_text(front_md))
}

/// `text` with every run of whitespace as one space, and none at the ends.
fn collapse_whitespace(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for word in text.split_whitespace() {
        if !out.is_empty() {
            out.push(' ');
        }
        out.push_str(word);
    }
    out
}

/// How much of a front's key the phrase query holds, in UTF-8 bytes from
/// its first letter or digit. FTS5 reads the matches of every token of a
/// phrase, so a pasted front of thousands of words would cost as many
/// lookups (ADR 0050); the cards whose fronts start with the same first
/// 256 bytes are few, and Rust compares the rest.
const MAX_PHRASE_BYTES: usize = 256;

/// The FTS5 query for the fronts that start with `key`'s words in order:
/// one quoted phrase on the `front` column, anchored to the column's first
/// token (`^`), its quotes doubled, so nothing in it is ever FTS5 syntax.
/// A key longer than [`MAX_PHRASE_BYTES`] is cut, and its last word
/// becomes a prefix (`"…"*`), so a word cut in two still matches. `None`
/// if the key has no token character ([`starts_token`]): such a front has
/// nothing to look up, and gets no note (ADR 0072).
fn phrase_query(key: &str) -> Option<String> {
    // What comes before the first token character holds no token, so the
    // phrase is the same without it, the stored front's first token is the
    // phrase's first, and it never uses up the budget.
    let start = key.find(starts_token)?;
    let words = &key[start..];
    let (words, cut) = if words.len() > MAX_PHRASE_BYTES {
        let mut end = MAX_PHRASE_BYTES;
        while !words.is_char_boundary(end) {
            end -= 1;
        }
        (&words[..end], true)
    } else {
        (words, false)
    };
    // NUL cannot be quoted in an FTS5 string; it is no token character, so
    // as a space it separates the same tokens.
    let quoted = words.replace('\0', " ").replace('"', "\"\"");
    Some(format!(
        "front : ^ \"{quoted}\"{}",
        if cut { "*" } else { "" }
    ))
}

/// The longest front looked up, in bytes of its Markdown: 64 KiB. Parsing
/// and folding a front is what costs, about 90 ms for 1 MiB of dense
/// Markdown in a release build (330 ms in a debug one), on the editor's
/// main thread at every pause in typing; 64 KiB takes a few
/// milliseconds. A longer front is no note's business: such a card is
/// not typed twice by mistake (ADR 0072).
pub const MAX_SAME_FRONT_BYTES: usize = 64 * 1024;

impl Store {
    /// The live cards, in every live deck (archived ones included), whose
    /// front reads the same as `front_md` (`front_key`), other than card
    /// `except_id`: those in decks not archived first, then the newest
    /// first. Empty for a front without a letter, digit or private-use
    /// character, and for one longer than [`MAX_SAME_FRONT_BYTES`], which
    /// is not even parsed.
    ///
    /// One FTS5 phrase query on the search index (ADR 0035) finds the
    /// fronts that start with the words in order; of those, only the ones whose
    /// whole key is the same are kept.
    ///
    /// # Errors
    /// SQLite errors.
    pub fn cards_with_front(
        &self,
        front_md: &str,
        except_id: Option<&str>,
    ) -> Result<Vec<SameFront>> {
        if front_md.len() > MAX_SAME_FRONT_BYTES {
            return Ok(Vec::new());
        }
        same_front_in(&self.conn, &front_key(front_md), except_id)
    }
}

/// [`Store::cards_with_front`] for a key already made.
fn same_front_in(conn: &Connection, key: &str, except_id: Option<&str>) -> Result<Vec<SameFront>> {
    let Some(phrase) = phrase_query(key) else {
        return Ok(Vec::new());
    };
    // No ORDER BY: SQLite would copy every matching row, its whole front
    // included, into a sorter before the first one is compared (a 1 MiB
    // front 256 times over doubled the lookup's time, ADR 0073). Each row
    // is compared as it comes and only the matches are sorted, in Rust.
    let mut stmt = conn.prepare(
        "SELECT c.id, c.deck_id, d.name, d.archived_at IS NOT NULL, c.seq, f.front
         FROM cards_fts f
         JOIN cards c ON c.seq = f.rowid AND c.deleted_at IS NULL
         JOIN decks d ON d.id = c.deck_id AND d.deleted_at IS NULL
         WHERE cards_fts MATCH ?1 AND c.id IS NOT ?2",
    )?;
    let mut rows = stmt.query(rusqlite::params![phrase, except_id])?;
    let mut found = Vec::new();
    while let Some(r) = rows.next()? {
        // Compared word by word, never copied: `key` is not empty and
        // has single spaces only.
        let stored = r.get_ref(5)?.as_str().map_err(rusqlite::Error::from)?;
        if !stored.split_whitespace().eq(key.split(' ')) {
            continue;
        }
        let seq: i64 = r.get(4)?;
        let same = SameFront {
            card_id: r.get(0)?,
            deck_id: r.get(1)?,
            deck_name: r.get(2)?,
            archived: r.get(3)?,
        };
        found.push((seq, same));
    }
    // Decks not archived first, then the newest card (the largest `seq`).
    found.sort_by_key(|(seq, same)| (same.archived, std::cmp::Reverse(*seq)));
    Ok(found.into_iter().map(|(_, same)| same).collect())
}

#[cfg(test)]
mod tests;
