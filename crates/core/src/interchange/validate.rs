//! Checks on a parsed `cards.json` that need no database: the file is
//! untrusted, so every id, reference, grade, timestamp and text is checked
//! before anything is compared or written.

use std::collections::{HashMap, HashSet};

use super::format::{Document, FORMAT_VERSION, Header};
use super::{FieldProblem, ImportProblem, Limits};
use crate::domain::{Grade, deck_name_key, has_nul};
use crate::id::uuid_v7_ms;

/// 9999-12-31T23:59:59.999Z, the latest timestamp a file may hold.
const MAX_TIMESTAMP: i64 = 253_402_300_799_999;

/// Parses `json` into a checked [`Document`].
pub(super) fn parse(json: &[u8], limits: &Limits) -> Result<Document, ImportProblem> {
    let header: Header = serde_json::from_slice(json).map_err(|e| json_problem(&e))?;
    if header.format_version != i64::from(FORMAT_VERSION) {
        return Err(ImportProblem::UnsupportedFormatVersion {
            found: header.format_version,
        });
    }
    let document: Document = serde_json::from_slice(json).map_err(|e| json_problem(&e))?;
    check(&document, limits)?;
    Ok(document)
}

/// Only the kind of error and its place: serde's message can quote any
/// amount of the file's text (a string where a number belongs).
fn json_problem(err: &serde_json::Error) -> ImportProblem {
    let kind = match err.classify() {
        serde_json::error::Category::Io => "cannot be read",
        serde_json::error::Category::Syntax => "invalid JSON",
        serde_json::error::Category::Data => "unexpected field, value or type",
        serde_json::error::Category::Eof => "ends too early",
    };
    ImportProblem::Json(format!(
        "{kind} at line {}, column {}",
        err.line(),
        err.column()
    ))
}

/// Every rule a file must follow, in file order: top level, decks, cards,
/// then review events.
pub(super) fn check(doc: &Document, limits: &Limits) -> Result<(), ImportProblem> {
    if !timestamp_ok(doc.exported_at) {
        return Err(ImportProblem::InvalidTopLevel {
            field: "exported_at",
            reason: FieldProblem::TimestampOutOfRange,
        });
    }
    if !doc.settings.is_empty() {
        return Err(ImportProblem::UnsupportedSettings);
    }
    if !doc.media.is_empty() {
        return Err(ImportProblem::UnsupportedMedia);
    }

    // Deck id -> whether the deck is deleted.
    let mut decks: HashMap<&str, bool> = HashMap::with_capacity(doc.decks.len());
    let mut live_names: HashSet<String> = HashSet::new();
    for (index, d) in doc.decks.iter().enumerate() {
        let at = |field, reason| invalid("decks", index, field, reason);
        check_id(&d.id).map_err(|r| at("id", r))?;
        if decks.insert(&d.id, d.deleted_at.is_some()).is_some() {
            return Err(at("id", FieldProblem::DuplicateId));
        }
        if d.name.trim().is_empty() {
            return Err(at("name", FieldProblem::Empty));
        }
        if d.name.trim() != d.name {
            return Err(at("name", FieldProblem::NotTrimmed));
        }
        if d.name.len() > limits.deck_name {
            return Err(at("name", FieldProblem::TooLong));
        }
        if has_nul(&d.name) {
            return Err(at("name", FieldProblem::ContainsNul));
        }
        check_times(d.created_at, d.updated_at, d.deleted_at)
            .map_err(|f| at(f, FieldProblem::TimestampOutOfRange))?;
        if d.deleted_at.is_none() && !live_names.insert(deck_name_key(&d.name)) {
            return Err(at("name", FieldProblem::DuplicateDeckName));
        }
    }

    let mut cards: HashSet<&str> = HashSet::with_capacity(doc.cards.len());
    for (index, c) in doc.cards.iter().enumerate() {
        let at = |field, reason| invalid("cards", index, field, reason);
        check_id(&c.id).map_err(|r| at("id", r))?;
        if !cards.insert(&c.id) {
            return Err(at("id", FieldProblem::DuplicateId));
        }
        let Some(&deck_deleted) = decks.get(c.deck_id.as_str()) else {
            return Err(at("deck_id", FieldProblem::UnknownReference));
        };
        if deck_deleted && c.deleted_at.is_none() {
            return Err(at("deleted_at", FieldProblem::LiveCardInDeletedDeck));
        }
        if c.front_md.trim().is_empty() {
            return Err(at("front_md", FieldProblem::Empty));
        }
        if c.front_md.len() > limits.card_text {
            return Err(at("front_md", FieldProblem::TooLong));
        }
        if c.back_md.len() > limits.card_text {
            return Err(at("back_md", FieldProblem::TooLong));
        }
        if has_nul(&c.front_md) {
            return Err(at("front_md", FieldProblem::ContainsNul));
        }
        if has_nul(&c.back_md) {
            return Err(at("back_md", FieldProblem::ContainsNul));
        }
        check_times(c.created_at, c.updated_at, c.deleted_at)
            .map_err(|f| at(f, FieldProblem::TimestampOutOfRange))?;
    }

    let mut events: HashSet<&str> = HashSet::with_capacity(doc.review_events.len());
    for (index, e) in doc.review_events.iter().enumerate() {
        let at = |field, reason| invalid("review_events", index, field, reason);
        check_id(&e.id).map_err(|r| at("id", r))?;
        if !events.insert(&e.id) {
            return Err(at("id", FieldProblem::DuplicateId));
        }
        if !cards.contains(e.card_id.as_str()) {
            return Err(at("card_id", FieldProblem::UnknownReference));
        }
        if !timestamp_ok(e.reviewed_at) {
            return Err(at("reviewed_at", FieldProblem::TimestampOutOfRange));
        }
        if e.grade.parse::<Grade>().is_err() {
            return Err(at("grade", FieldProblem::InvalidGrade));
        }
        if e.duration_ms < 0 {
            return Err(at("duration_ms", FieldProblem::Negative));
        }
    }
    Ok(())
}

fn invalid(
    table: &'static str,
    index: usize,
    field: &'static str,
    reason: FieldProblem,
) -> ImportProblem {
    ImportProblem::InvalidField {
        table,
        index,
        field,
        reason,
    }
}

/// A reference that is not a valid id cannot resolve either, so only row
/// ids are checked for their form; references are checked by lookup.
///
/// The id's own timestamp must be in range too: `IdGen` issues every new
/// id above the largest stored one, so a stored id near the top of the
/// `UUIDv7` range (year 10889) would leave it no room and make every later
/// write fail with `IdsExhausted`, which nothing could repair because rows
/// are never removed. Up to the year 9999 leaves centuries of room.
fn check_id(id: &str) -> Result<(), FieldProblem> {
    match uuid_v7_ms(id) {
        None => Err(FieldProblem::NotUuidV7),
        Some(ms) if !timestamp_ok(ms) => Err(FieldProblem::TimestampOutOfRange),
        Some(_) => Ok(()),
    }
}

fn timestamp_ok(ms: i64) -> bool {
    (0..=MAX_TIMESTAMP).contains(&ms)
}

/// The first of the three timestamps that is out of range, by field name.
fn check_times(created: i64, updated: i64, deleted: Option<i64>) -> Result<(), &'static str> {
    if !timestamp_ok(created) {
        return Err("created_at");
    }
    if !timestamp_ok(updated) {
        return Err("updated_at");
    }
    if deleted.is_some_and(|d| !timestamp_ok(d)) {
        return Err("deleted_at");
    }
    Ok(())
}
