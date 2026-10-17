//! Importing a `.rappel` file: plan (check and compare, no writes), then
//! apply (back up, then one transaction).

use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::fs::{self, File};
use std::io::Read;
use std::path::Path;

use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};

use super::format::{CardRecord, DeckRecord, Document, EventRecord, SettingsRecord};
use super::{
    Conflict, ImportPlan, ImportProblem, ImportReport, ImportSummary, Limits, SettingsImport,
    archive, validate,
};
use crate::domain::{deck_name_key, name_key};
use crate::error::{Error, Result};
use crate::storage::{
    Store, StoredSettings, create_backup, prune_backups, rebuild_cards, replace_settings,
    scheduler_in, stored_settings,
};
use crate::time::now_ms;

impl Store {
    /// Reads and checks the `.rappel` file `path` and compares it with the
    /// database. Writes nothing.
    ///
    /// A file that is not a valid export is refused with
    /// [`Error::ImportRefused`], saying what is wrong and where. A valid
    /// file gives a plan whose [`ImportPlan::summary`] counts the rows that
    /// would be added, those that would be brought back (deleted here,
    /// live in the file, otherwise the same) and those already present,
    /// and lists every conflict (a row with the same id but different
    /// content, a live deck name already used by another live deck, or an
    /// undo of an entry the database has already undone). A tag with the
    /// name of a database tag is mapped onto it, not a conflict. It also says whether the file's
    /// settings would be applied: only into a database without decks.
    /// Pass the plan to [`Self::apply_import`].
    ///
    /// # Errors
    /// [`Error::ImportRefused`], IO errors reading `path`, SQLite errors.
    pub fn plan_import(&self, path: &Path) -> Result<ImportPlan> {
        self.plan_import_with(path, &Limits::DEFAULT)
    }

    /// [`Self::plan_import`] with explicit limits, for tests.
    ///
    /// Each buffer is dropped as soon as the next one is made, so the peak
    /// is the larger of archive + JSON and JSON + document, not their sum
    /// (ADR 0025).
    pub(crate) fn plan_import_with(&self, path: &Path, limits: &Limits) -> Result<ImportPlan> {
        let json = {
            let bytes = read_limited(path, limits.archive)?;
            archive::read_cards_json(&bytes, limits).map_err(Error::ImportRefused)?
        };
        self.plan_import_json(json, limits)
    }

    /// [`Self::plan_import`] on the archive bytes.
    #[cfg(test)]
    pub(crate) fn plan_import_bytes(&self, bytes: &[u8], limits: &Limits) -> Result<ImportPlan> {
        let json = archive::read_cards_json(bytes, limits).map_err(Error::ImportRefused)?;
        self.plan_import_json(json, limits)
    }

    /// [`Self::plan_import`] on the decompressed `cards.json`, which is
    /// dropped once parsed.
    fn plan_import_json(&self, json: Vec<u8>, limits: &Limits) -> Result<ImportPlan> {
        let document = validate::parse(&json, limits).map_err(Error::ImportRefused)?;
        drop(json);
        let tx = self.conn.unchecked_transaction()?;
        let classified = classify(&tx, &document)?;
        // Read only: ending the transaction cannot lose anything.
        tx.finish()?;
        Ok(ImportPlan {
            document,
            classified,
        })
    }

    /// Adds the plan's new decks, cards, tags, card tags and log entries
    /// to the database, brings back the deleted decks and cards that are
    /// live in the file, and applies the file's settings if the plan says
    /// so (before the schedules are rebuilt, so they use them). A deck brought back also brings back the cards deleted with
    /// it, as [`Self::undelete_deck`] does, unless the file has them.
    ///
    /// Nothing is written if the plan has conflicts, or if every row is
    /// already present (then no backup is made either, so importing a file
    /// twice changes nothing). Otherwise the database is backed up first
    /// (no backup, no import), then, in one transaction, the file is
    /// compared again (refused if the database changed since the plan),
    /// the rows are brought back and inserted, and the schedule cache of
    /// every card that got reviews is rebuilt from its log. A card brought
    /// back keeps its schedule, which deleting it did not touch.
    ///
    /// # Errors
    /// [`Error::ImportConflicts`], [`Error::ImportPlanStale`],
    /// [`Error::NoBackupDir`], backup, scheduler and SQLite errors. On
    /// error nothing is written.
    pub fn apply_import(&mut self, plan: &ImportPlan) -> Result<ImportReport> {
        self.apply_import_at(plan, now_ms())
    }

    /// [`Self::apply_import`] at time `now` (names the backup).
    pub(crate) fn apply_import_at(&mut self, plan: &ImportPlan, now: i64) -> Result<ImportReport> {
        let summary = &plan.classified.summary;
        if !summary.conflicts.is_empty() {
            return Err(Error::ImportConflicts(summary.conflicts.len()));
        }
        if summary.is_noop() {
            return Ok(ImportReport {
                decks: 0,
                cards: 0,
                tags: 0,
                card_tags: 0,
                review_events: 0,
                settings_applied: false,
                revived_decks: 0,
                revived_cards: 0,
                backup: None,
            });
        }

        let backup_dir = self.backup_dir.clone().ok_or(Error::NoBackupDir)?;
        let backup = create_backup(&self.conn, &backup_dir, now)?;
        tracing::info!(path = %backup.display(), "database backed up before an import");
        if let Err(err) = prune_backups(&backup_dir, now, &[&backup]) {
            // As at startup: a failed prune only leaves extra backups.
            tracing::warn!("pruning old backups failed: {err}");
        }

        let doc = &plan.document;
        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let rows = classify(&tx, doc)?;
        // The same file, so equal positions are the same rows: the plan
        // the user confirmed is exactly what is written. Any change here
        // since (a row added, edited, deleted or brought back) that
        // changes what would be written refuses the import.
        if rows != plan.classified {
            return Err(Error::ImportPlanStale);
        }
        // Decks first, so a new card can go into a revived deck.
        revive_rows(&tx, doc, &rows)?;
        let reviewed = insert_rows(&tx, doc, &rows)?;
        let settings_applied = summary.settings == SettingsImport::Applied;
        if settings_applied {
            replace_settings(&tx, &stored_of(&doc.settings))?;
        }
        rebuild_cards(&tx, &scheduler_in(&tx)?, reviewed)?;
        tx.commit()?;
        tracing::info!(
            decks = summary.decks.new,
            cards = summary.cards.new,
            tags = summary.tags.new,
            card_tags = summary.card_tags.new,
            review_events = summary.review_events.new,
            settings_applied,
            revived_decks = summary.decks.revived,
            revived_cards = summary.cards.revived,
            "import applied"
        );
        Ok(ImportReport {
            decks: summary.decks.new,
            cards: summary.cards.new,
            tags: summary.tags.new,
            card_tags: summary.card_tags.new,
            review_events: summary.review_events.new,
            settings_applied,
            revived_decks: summary.decks.revived,
            revived_cards: summary.cards.revived,
            backup: Some(backup),
        })
    }
}

/// Reads at most `limit` bytes of `path`; a larger file is refused.
fn read_limited(path: &Path, limit: u64) -> Result<Vec<u8>> {
    let io_err = |source| Error::Io {
        path: path.to_path_buf(),
        source,
    };
    // Only a regular file: opening a FIFO blocks until something writes
    // to it (the import would never end), and a device such as /dev/zero
    // never ends. Checked before opening, through symbolic links, and
    // again on the open file. A FIFO swapped in between the two checks
    // would still block; the path comes from the user's own file chooser,
    // so that race is accepted (ADR 0025).
    let not_a_file = || Error::ImportRefused(ImportProblem::NotAFile);
    if !fs::metadata(path).map_err(io_err)?.is_file() {
        return Err(not_a_file());
    }
    let file = File::open(path).map_err(io_err)?;
    let metadata = file.metadata().map_err(io_err)?;
    if !metadata.is_file() {
        return Err(not_a_file());
    }
    let too_large = Error::ImportRefused(ImportProblem::FileTooLarge { limit });
    if metadata.len() > limit {
        return Err(too_large);
    }
    let mut bytes = Vec::new();
    // The file may grow after the size check: read one byte past the limit.
    file.take(limit.saturating_add(1))
        .read_to_end(&mut bytes)
        .map_err(io_err)?;
    if bytes.len() as u64 > limit {
        return Err(too_large);
    }
    Ok(bytes)
}

/// How a document compares with the database: the summary, the positions
/// of the rows to add and to bring back, the ids of the cards that come
/// back with a revived deck but are not in the file, and the file's tags
/// that are mapped onto a database tag of the same name.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(super) struct Classified {
    pub(super) summary: ImportSummary,
    new_decks: Vec<usize>,
    new_cards: Vec<usize>,
    new_tags: Vec<usize>,
    new_card_tags: Vec<usize>,
    new_events: Vec<usize>,
    revived_decks: Vec<usize>,
    revived_cards: Vec<usize>,
    batch_cards: Vec<String>,
    /// File tag id → the database's tag id with the same name key.
    mapped_tags: BTreeMap<String, String>,
}

impl Classified {
    /// The database tag id a file tag id stands for.
    fn tag_id<'a>(&'a self, file_id: &'a str) -> &'a str {
        self.mapped_tags
            .get(file_id)
            .map_or(file_id, String::as_str)
    }
}

/// What to do with a deck or card of the file whose id is in the database.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Outcome {
    /// Nothing: it is already here.
    Present,
    /// Clear the database row's `deleted_at`, which is this stamp.
    Revive(i64),
    /// Refuse the whole import.
    Conflict,
}

/// Compares a row of the file with the database's row of the same id.
/// `rest_equal` says whether every field but `deleted_at` is the same.
/// Only a difference in `deleted_at` can be merged: deleted here and live
/// in the file brings the row back; live here and deleted in the file
/// leaves it live, as an import never deletes. Deleted on both sides is
/// already present, whatever the stamps: merging it would change nothing.
/// Any other difference is a conflict.
pub(super) fn compare(
    rest_equal: bool,
    stored_deleted: Option<i64>,
    file_deleted: Option<i64>,
) -> Outcome {
    if !rest_equal {
        return Outcome::Conflict;
    }
    match (stored_deleted, file_deleted) {
        (Some(stamp), None) => Outcome::Revive(stamp),
        // Live here, or deleted on both sides.
        (None, _) | (Some(_), Some(_)) => Outcome::Present,
    }
}

/// Unknown id: new; otherwise as [`compare`] says. A revived deck also
/// revives the database's cards of the same delete batch (same deck, same
/// `deleted_at`, as [`Store::undelete_deck`]) that the file does not have;
/// those the file has are decided by their own row. Then live deck names
/// against the database's other live decks, which covers revived decks.
/// Conflicts come in file order: decks, cards, tags, events, then deck
/// names. Last, the settings.
fn classify(conn: &Connection, doc: &Document) -> Result<Classified> {
    let mut rows = Classified::default();
    let batches = classify_decks(conn, doc, &mut rows)?;
    classify_cards(conn, doc, &mut rows)?;
    add_batch_cards(conn, doc, &batches, &mut rows)?;
    classify_tags(conn, doc, &mut rows)?;
    classify_card_tags(conn, doc, &mut rows)?;
    classify_events(conn, doc, &mut rows)?;
    check_deck_names(conn, doc, &mut rows.summary)?;
    rows.summary.settings = classify_settings(conn, &doc.settings)?;
    Ok(rows)
}

/// A tag with the same id is present if it has the same name, else a
/// conflict. A tag whose id is not here but whose name key a database tag
/// has is that tag (two machines that each created "maths"): it is mapped
/// onto the database's id, counted as present, and the file's card tags
/// that name it attach to the database's tag. The database keeps its
/// own spelling. Else the tag is new.
fn classify_tags(conn: &Connection, doc: &Document, rows: &mut Classified) -> Result<()> {
    let mut by_id = conn.prepare("SELECT name FROM tags WHERE id = ?1")?;
    let mut by_key = conn.prepare("SELECT id FROM tags WHERE name_key = ?1")?;
    for (i, t) in doc.tags.iter().enumerate() {
        let stored: Option<String> = by_id.query_row([&t.id], |r| r.get(0)).optional()?;
        match stored {
            Some(name) if name == t.name => rows.summary.tags.present += 1,
            Some(_) => rows
                .summary
                .conflicts
                .push(Conflict::TagDiffers(t.id.clone())),
            None => {
                let existing: Option<String> = by_key
                    .query_row([name_key(&t.name)], |r| r.get(0))
                    .optional()?;
                if let Some(existing_id) = existing {
                    rows.summary.tags.present += 1;
                    rows.mapped_tags.insert(t.id.clone(), existing_id);
                } else {
                    rows.summary.tags.new += 1;
                    rows.new_tags.push(i);
                }
            }
        }
    }
    Ok(())
}

/// A card tag is present or new, once its tag is mapped
/// ([`classify_tags`]); it has no content to differ.
fn classify_card_tags(conn: &Connection, doc: &Document, rows: &mut Classified) -> Result<()> {
    let mut stmt = conn.prepare("SELECT 1 FROM card_tags WHERE card_id = ?1 AND tag_id = ?2")?;
    for (i, ct) in doc.card_tags.iter().enumerate() {
        if stmt.exists(params![ct.card_id, rows.tag_id(&ct.tag_id)])? {
            rows.summary.card_tags.present += 1;
        } else {
            rows.summary.card_tags.new += 1;
            rows.new_card_tags.push(i);
        }
    }
    Ok(())
}

/// The file's settings go in only when the database has no decks at all,
/// deleted ones included (ADR 0027): moving to a new machine brings them,
/// a merge never changes them. Applied, they replace the stored ones, so
/// the database ends with the settings the file's database had, an
/// empty `settings` (all defaults) included.
fn classify_settings(conn: &Connection, file: &SettingsRecord) -> Result<SettingsImport> {
    if stored_of(file) == stored_settings(conn)? {
        return Ok(SettingsImport::Unchanged);
    }
    let has_decks: bool =
        conn.query_row("SELECT EXISTS (SELECT 1 FROM decks)", [], |r| r.get(0))?;
    Ok(if has_decks {
        SettingsImport::Kept
    } else {
        SettingsImport::Applied
    })
}

fn stored_of(s: &SettingsRecord) -> StoredSettings {
    StoredSettings {
        desired_retention: s.desired_retention,
        max_new_per_day: s.max_new_per_day,
        max_reviews_per_day: s.max_reviews_per_day,
        day_start_hour: s.day_start_hour,
    }
}

/// Classifies the decks. Returns each revived deck's id with its
/// `deleted_at` here, the stamp of its delete batch.
fn classify_decks<'d>(
    conn: &Connection,
    doc: &'d Document,
    rows: &mut Classified,
) -> Result<Vec<(&'d str, i64)>> {
    let mut batches = Vec::new();
    let mut stmt = conn
        .prepare("SELECT id, name, created_at, updated_at, deleted_at FROM decks WHERE id = ?1")?;
    for (i, d) in doc.decks.iter().enumerate() {
        let stored = stmt
            .query_row([&d.id], |r| {
                Ok(DeckRecord {
                    id: r.get(0)?,
                    name: r.get(1)?,
                    created_at: r.get(2)?,
                    updated_at: r.get(3)?,
                    deleted_at: r.get(4)?,
                })
            })
            .optional()?;
        let Some(s) = stored else {
            rows.summary.decks.new += 1;
            rows.new_decks.push(i);
            continue;
        };
        let stored_deleted = s.deleted_at;
        let rest_equal = DeckRecord {
            deleted_at: d.deleted_at,
            ..s
        } == *d;
        match compare(rest_equal, stored_deleted, d.deleted_at) {
            Outcome::Present => rows.summary.decks.present += 1,
            Outcome::Revive(stamp) => {
                rows.summary.decks.revived += 1;
                rows.revived_decks.push(i);
                batches.push((d.id.as_str(), stamp));
            }
            Outcome::Conflict => rows
                .summary
                .conflicts
                .push(Conflict::DeckDiffers(d.id.clone())),
        }
    }
    Ok(batches)
}

fn classify_cards(conn: &Connection, doc: &Document, rows: &mut Classified) -> Result<()> {
    let mut stmt = conn.prepare(
        "SELECT id, deck_id, front_md, back_md, created_at, updated_at, suspended, deleted_at
         FROM cards WHERE id = ?1",
    )?;
    for (i, c) in doc.cards.iter().enumerate() {
        let stored = stmt
            .query_row([&c.id], |r| {
                Ok(CardRecord {
                    id: r.get(0)?,
                    deck_id: r.get(1)?,
                    front_md: r.get(2)?,
                    back_md: r.get(3)?,
                    created_at: r.get(4)?,
                    updated_at: r.get(5)?,
                    suspended: r.get(6)?,
                    deleted_at: r.get(7)?,
                })
            })
            .optional()?;
        let Some(s) = stored else {
            rows.summary.cards.new += 1;
            rows.new_cards.push(i);
            continue;
        };
        let stored_deleted = s.deleted_at;
        // The database keeps its own `suspended`: an import never changes
        // how a live card is studied (ADR 0035).
        let rest_equal = CardRecord {
            deleted_at: c.deleted_at,
            suspended: c.suspended,
            ..s
        } == *c;
        match compare(rest_equal, stored_deleted, c.deleted_at) {
            Outcome::Present => rows.summary.cards.present += 1,
            // The stamp matters only for a deck, where it marks the batch.
            Outcome::Revive(_) => {
                rows.summary.cards.revived += 1;
                rows.revived_cards.push(i);
            }
            Outcome::Conflict => rows
                .summary
                .conflicts
                .push(Conflict::CardDiffers(c.id.clone())),
        }
    }
    Ok(())
}

/// Adds the cards deleted with each revived deck (same deck, same stamp)
/// that are not in the file.
fn add_batch_cards(
    conn: &Connection,
    doc: &Document,
    batches: &[(&str, i64)],
    rows: &mut Classified,
) -> Result<()> {
    let in_file: HashSet<&str> = doc.cards.iter().map(|c| c.id.as_str()).collect();
    let mut stmt =
        conn.prepare("SELECT id FROM cards WHERE deck_id = ?1 AND deleted_at = ?2 ORDER BY id")?;
    for &(deck_id, stamp) in batches {
        let ids = stmt.query_map(params![deck_id, stamp], |r| r.get::<_, String>(0))?;
        for id in ids {
            let id = id?;
            if !in_file.contains(id.as_str()) {
                rows.summary.cards.revived += 1;
                rows.batch_cards.push(id);
            }
        }
    }
    Ok(())
}

/// An entry with the same id is present if it is the same, else a
/// conflict. A new undo of an entry that the database has already undone
/// (with another undo) is a conflict too: an entry is undone once.
fn classify_events(conn: &Connection, doc: &Document, rows: &mut Classified) -> Result<()> {
    let mut stmt = conn.prepare(
        "SELECT id, card_id, kind, reviewed_at, grade, duration_ms, undoes
         FROM review_events WHERE id = ?1",
    )?;
    let mut undone_by = conn.prepare("SELECT id FROM review_events WHERE undoes = ?1")?;
    for (i, e) in doc.review_events.iter().enumerate() {
        let stored = stmt
            .query_row([&e.id], |r| {
                Ok(EventRecord {
                    id: r.get(0)?,
                    card_id: r.get(1)?,
                    kind: r.get(2)?,
                    reviewed_at: r.get(3)?,
                    grade: r.get(4)?,
                    duration_ms: r.get(5)?,
                    undoes: r.get(6)?,
                })
            })
            .optional()?;
        match stored {
            None => {
                let existing: Option<String> = match &e.undoes {
                    Some(target) => undone_by.query_row([target], |r| r.get(0)).optional()?,
                    None => None,
                };
                if let Some(existing_id) = existing {
                    rows.summary.conflicts.push(Conflict::AlreadyUndone {
                        event_id: e.id.clone(),
                        existing_id,
                    });
                } else {
                    rows.summary.review_events.new += 1;
                    rows.new_events.push(i);
                }
            }
            Some(s) if s == *e => rows.summary.review_events.present += 1,
            Some(_) => rows
                .summary
                .conflicts
                .push(Conflict::EventDiffers(e.id.clone())),
        }
    }
    Ok(())
}

/// Each live deck of the file (new, present or revived) against the
/// database's other live decks.
fn check_deck_names(conn: &Connection, doc: &Document, summary: &mut ImportSummary) -> Result<()> {
    let mut stmt = conn
        .prepare("SELECT id FROM decks WHERE name_key = ?1 AND deleted_at IS NULL AND id <> ?2")?;
    for d in doc.decks.iter().filter(|d| d.deleted_at.is_none()) {
        let existing: Option<String> = stmt
            .query_row(params![deck_name_key(&d.name), d.id], |r| r.get(0))
            .optional()?;
        if let Some(existing_id) = existing {
            summary.conflicts.push(Conflict::DeckNameTaken {
                deck_id: d.id.clone(),
                existing_id,
            });
        }
    }
    Ok(())
}

/// Brings back the revived decks, then the revived cards. `updated_at` is
/// not touched, as deleting did not touch it.
fn revive_rows(conn: &Connection, doc: &Document, rows: &Classified) -> Result<()> {
    let mut deck = conn
        .prepare("UPDATE decks SET deleted_at = NULL WHERE id = ?1 AND deleted_at IS NOT NULL")?;
    for d in rows.revived_decks.iter().map(|&i| &doc.decks[i]) {
        deck.execute([&d.id])?;
    }
    let mut card = conn
        .prepare("UPDATE cards SET deleted_at = NULL WHERE id = ?1 AND deleted_at IS NOT NULL")?;
    let from_file = rows.revived_cards.iter().map(|&i| doc.cards[i].id.as_str());
    for id in from_file.chain(rows.batch_cards.iter().map(String::as_str)) {
        card.execute([id])?;
    }
    Ok(())
}

/// Inserts the rows at the given positions of `doc`.
fn insert_rows<'d>(
    conn: &Connection,
    doc: &'d Document,
    rows: &Classified,
) -> Result<BTreeSet<&'d str>> {
    let mut deck = conn.prepare(
        "INSERT INTO decks (id, name, name_key, created_at, updated_at, deleted_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
    )?;
    for d in rows.new_decks.iter().map(|&i| &doc.decks[i]) {
        deck.execute(params![
            d.id,
            d.name,
            deck_name_key(&d.name),
            d.created_at,
            d.updated_at,
            d.deleted_at
        ])?;
    }
    let mut card = conn.prepare(
        "INSERT INTO cards (id, deck_id, front_md, back_md, created_at, updated_at, suspended, deleted_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
    )?;
    for c in rows.new_cards.iter().map(|&i| &doc.cards[i]) {
        card.execute(params![
            c.id,
            c.deck_id,
            c.front_md,
            c.back_md,
            c.created_at,
            c.updated_at,
            c.suspended,
            c.deleted_at
        ])?;
        crate::storage::index_card_in(conn, &c.id, &c.front_md, &c.back_md)?;
    }
    let mut tag = conn.prepare("INSERT INTO tags (id, name, name_key) VALUES (?1, ?2, ?3)")?;
    for t in rows.new_tags.iter().map(|&i| &doc.tags[i]) {
        tag.execute(params![t.id, t.name, name_key(&t.name)])?;
    }
    let mut card_tag = conn.prepare("INSERT INTO card_tags (card_id, tag_id) VALUES (?1, ?2)")?;
    for ct in rows.new_card_tags.iter().map(|&i| &doc.card_tags[i]) {
        card_tag.execute(params![ct.card_id, rows.tag_id(&ct.tag_id)])?;
    }
    // In file order, which is by id: an undo's target (smaller id, checked
    // by validation) is inserted before it, or is already here.
    let mut event = conn.prepare(
        "INSERT INTO review_events (id, card_id, kind, reviewed_at, grade, duration_ms, undoes)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
    )?;
    let mut reviewed = BTreeSet::new();
    let mut new_events: Vec<&EventRecord> = rows
        .new_events
        .iter()
        .map(|&i| &doc.review_events[i])
        .collect();
    // The file's list should be sorted by id; sorting again costs little
    // and does not trust that.
    new_events.sort_by(|a, b| a.id.cmp(&b.id));
    for e in new_events {
        event.execute(params![
            e.id,
            e.card_id,
            e.kind,
            e.reviewed_at,
            e.grade,
            e.duration_ms,
            e.undoes
        ])?;
        reviewed.insert(e.card_id.as_str());
    }
    Ok(reviewed)
}
