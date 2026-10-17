//! Checks on a parsed `cards.json` that need no database: the file is
//! untrusted, so every id, reference, kind, grade, timestamp, text and
//! setting is checked before anything is compared or written.

use std::collections::{HashMap, HashSet};

use super::format::{Document, EARLY_TEST_VERSION, EventRecord, FORMAT_VERSION, Header};
use super::{FieldProblem, ImportProblem, Limits};
use crate::domain::{Grade, Settings, deck_name_key, has_nul, name_key};
use crate::id::uuid_v7_ms;

/// 9999-12-31T23:59:59.999Z, the latest timestamp a file may hold.
const MAX_TIMESTAMP: i64 = 253_402_300_799_999;

/// Parses `json` into a checked [`Document`].
pub(super) fn parse(json: &[u8], limits: &Limits) -> Result<Document, ImportProblem> {
    let header: Header = serde_json::from_slice(json).map_err(|e| json_problem(&e))?;
    if header.format_version == EARLY_TEST_VERSION {
        return Err(ImportProblem::EarlyTestVersion);
    }
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
/// tags, card tags, review events, then settings. The first broken rule is
/// the one reported.
pub(super) fn check(doc: &Document, limits: &Limits) -> Result<(), ImportProblem> {
    check_top_level(doc)?;
    let decks = check_decks(doc, limits)?;
    let cards = check_cards(doc, &decks, limits)?;
    let tags = check_tags(doc, limits)?;
    check_card_tags(doc, &cards, &tags)?;
    check_review_events(doc, &cards)?;
    check_settings(doc)
}

fn check_top_level(doc: &Document) -> Result<(), ImportProblem> {
    if !timestamp_ok(doc.exported_at) {
        return Err(ImportProblem::InvalidTopLevel {
            field: "exported_at",
            reason: FieldProblem::TimestampOutOfRange,
        });
    }
    if !doc.media.is_empty() {
        return Err(ImportProblem::UnsupportedMedia);
    }
    Ok(())
}

/// The decks' rules. Returns each deck id with whether the deck is deleted.
fn check_decks<'d>(
    doc: &'d Document,
    limits: &Limits,
) -> Result<HashMap<&'d str, bool>, ImportProblem> {
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
    Ok(decks)
}

/// The cards' rules, given the decks from [`check_decks`]. Returns the card
/// ids.
fn check_cards<'d>(
    doc: &'d Document,
    decks: &HashMap<&str, bool>,
    limits: &Limits,
) -> Result<HashSet<&'d str>, ImportProblem> {
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
    Ok(cards)
}

/// The tags' rules. Returns the tag ids.
fn check_tags<'d>(doc: &'d Document, limits: &Limits) -> Result<HashSet<&'d str>, ImportProblem> {
    let mut tags: HashSet<&str> = HashSet::with_capacity(doc.tags.len());
    let mut keys: HashSet<String> = HashSet::with_capacity(doc.tags.len());
    for (index, t) in doc.tags.iter().enumerate() {
        let at = |field, reason| invalid("tags", index, field, reason);
        check_id(&t.id).map_err(|r| at("id", r))?;
        if !tags.insert(&t.id) {
            return Err(at("id", FieldProblem::DuplicateId));
        }
        // The rules of `check_tag_name`, each with its own problem.
        if t.name.trim().is_empty() {
            return Err(at("name", FieldProblem::Empty));
        }
        if t.name.trim() != t.name {
            return Err(at("name", FieldProblem::NotTrimmed));
        }
        if t.name.len() > limits.tag_name {
            return Err(at("name", FieldProblem::TooLong));
        }
        if has_nul(&t.name) {
            return Err(at("name", FieldProblem::ContainsNul));
        }
        if t.name.contains(',') {
            return Err(at("name", FieldProblem::ContainsComma));
        }
        if !keys.insert(name_key(&t.name)) {
            return Err(at("name", FieldProblem::DuplicateTagName));
        }
    }
    Ok(tags)
}

/// The card tags' rules, given the card and tag ids. The rows must come
/// in the order an export writes them, by card id then tag id, so a
/// repeated row is the one just before: no set of pairs is held, which
/// keeps the smallest rows of the format within the memory budget
/// (ADR 0025, row 9).
fn check_card_tags(
    doc: &Document,
    cards: &HashSet<&str>,
    tags: &HashSet<&str>,
) -> Result<(), ImportProblem> {
    let mut previous: Option<(&str, &str)> = None;
    for (index, ct) in doc.card_tags.iter().enumerate() {
        let at = |field, reason| invalid("card_tags", index, field, reason);
        if !cards.contains(ct.card_id.as_str()) {
            return Err(at("card_id", FieldProblem::UnknownReference));
        }
        if !tags.contains(ct.tag_id.as_str()) {
            return Err(at("tag_id", FieldProblem::UnknownReference));
        }
        let pair = (ct.card_id.as_str(), ct.tag_id.as_str());
        match previous {
            Some(p) if p == pair => return Err(at("tag_id", FieldProblem::DuplicateRow)),
            Some(p) if p > pair => return Err(at("card_id", FieldProblem::OutOfOrder)),
            _ => {}
        }
        previous = Some(pair);
    }
    Ok(())
}

/// The review events' rules, given the card ids from [`check_cards`]:
/// each row on its own first, then each undo against the entry it
/// cancels (ADR 0028), which may come anywhere in the list. Only the
/// entries an undo names are kept in a map, so a log of reviews needs no
/// more memory than before undos existed (ADR 0025, row 9).
fn check_review_events(doc: &Document, cards: &HashSet<&str>) -> Result<(), ImportProblem> {
    let mut events: HashSet<&str> = HashSet::with_capacity(doc.review_events.len());
    // Each named entry, filled in below; a name used twice is refused.
    let mut targets: HashMap<&str, Option<&EventRecord>> = HashMap::new();
    for (index, e) in doc.review_events.iter().enumerate() {
        let at = |field, reason| invalid("review_events", index, field, reason);
        check_id(&e.id).map_err(|r| at("id", r))?;
        if !events.insert(&e.id) {
            return Err(at("id", FieldProblem::DuplicateId));
        }
        if !cards.contains(e.card_id.as_str()) {
            return Err(at("card_id", FieldProblem::UnknownReference));
        }
        let (grade, undoes) = match e.kind.as_str() {
            "review" => (true, false),
            "reset" => (false, false),
            "undo" => (false, true),
            _ => return Err(at("kind", FieldProblem::InvalidKind)),
        };
        if !timestamp_ok(e.reviewed_at) {
            return Err(at("reviewed_at", FieldProblem::TimestampOutOfRange));
        }
        match (&e.grade, grade) {
            (Some(g), true) if g.parse::<Grade>().is_err() => {
                return Err(at("grade", FieldProblem::InvalidGrade));
            }
            (None, true) => return Err(at("grade", FieldProblem::Missing)),
            (Some(_), false) => return Err(at("grade", FieldProblem::NotAllowed)),
            _ => {}
        }
        if e.duration_ms < 0 {
            return Err(at("duration_ms", FieldProblem::Negative));
        }
        if !grade && e.duration_ms != 0 {
            return Err(at("duration_ms", FieldProblem::NotAllowed));
        }
        match (&e.undoes, undoes) {
            (None, true) => return Err(at("undoes", FieldProblem::Missing)),
            (Some(_), false) => return Err(at("undoes", FieldProblem::NotAllowed)),
            (Some(target), true) if targets.insert(target, None).is_some() => {
                return Err(at("undoes", FieldProblem::AlreadyUndone));
            }
            _ => {}
        }
    }
    if targets.is_empty() {
        return Ok(());
    }
    for e in &doc.review_events {
        if let Some(slot) = targets.get_mut(e.id.as_str()) {
            *slot = Some(e);
        }
    }
    for (index, e) in doc.review_events.iter().enumerate() {
        let Some(target_id) = &e.undoes else { continue };
        // An earlier review or reset of the same card: dated no later and
        // with a smaller id, so it is replayed (and inserted) first, as
        // the schema requires.
        let cancels = targets
            .get(target_id.as_str())
            .copied()
            .flatten()
            .is_some_and(|t| {
                t.card_id == e.card_id
                    && t.kind != "undo"
                    && t.reviewed_at <= e.reviewed_at
                    && t.id < e.id
            });
        if !cancels {
            return Err(invalid(
                "review_events",
                index,
                "undoes",
                FieldProblem::InvalidUndo,
            ));
        }
    }
    Ok(())
}

/// Each setting present is within its range.
fn check_settings(doc: &Document) -> Result<(), ImportProblem> {
    let s = &doc.settings;
    let out_of_range = |field| ImportProblem::InvalidTopLevel {
        field,
        reason: FieldProblem::OutOfRange,
    };
    if s.desired_retention
        .is_some_and(|v| Settings::check_retention(v).is_err())
    {
        return Err(out_of_range("settings.desired_retention"));
    }
    if s.max_new_per_day
        .is_some_and(|v| Settings::check_max_new(v).is_err())
    {
        return Err(out_of_range("settings.max_new_per_day"));
    }
    if s.max_reviews_per_day
        .is_some_and(|v| Settings::check_max_reviews(v).is_err())
    {
        return Err(out_of_range("settings.max_reviews_per_day"));
    }
    if s.day_start_hour
        .is_some_and(|v| Settings::check_day_start_hour(v).is_err())
    {
        return Err(out_of_range("settings.day_start_hour"));
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
