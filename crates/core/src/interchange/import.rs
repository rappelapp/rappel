//! Importing a `.cards` file: plan (check and compare, no writes), then
//! apply (back up, then one transaction).

use std::collections::BTreeSet;
use std::fs::File;
use std::io::Read;
use std::path::Path;

use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};

use super::format::{CardRecord, DeckRecord, Document, EventRecord};
use super::{
    Conflict, ImportPlan, ImportProblem, ImportReport, ImportSummary, Limits, archive, validate,
};
use crate::domain::deck_name_key;
use crate::error::{Error, Result};
use crate::storage::{KEEP_BACKUPS, Store, create_backup, prune_backups, rebuild_cards};
use crate::time::now_ms;

impl Store {
    /// Reads and checks the `.cards` file `path` and compares it with the
    /// database. Writes nothing.
    ///
    /// A file that is not a valid export is refused with
    /// [`Error::ImportRefused`], saying what is wrong and where. A valid
    /// file gives a plan whose [`ImportPlan::summary`] counts the rows that
    /// would be added and those already present, and lists every conflict
    /// (a row with the same id but different content, or a live deck name
    /// already used by another live deck). Pass the plan to
    /// [`Self::apply_import`].
    ///
    /// # Errors
    /// [`Error::ImportRefused`], IO errors reading `path`, SQLite errors.
    pub fn plan_import(&self, path: &Path) -> Result<ImportPlan> {
        self.plan_import_with(path, &Limits::DEFAULT)
    }

    /// [`Self::plan_import`] with explicit limits, for tests.
    pub(crate) fn plan_import_with(&self, path: &Path, limits: &Limits) -> Result<ImportPlan> {
        let bytes = read_limited(path, limits.archive)?;
        self.plan_import_bytes(&bytes, limits)
    }

    /// [`Self::plan_import`] on the archive bytes.
    pub(crate) fn plan_import_bytes(&self, bytes: &[u8], limits: &Limits) -> Result<ImportPlan> {
        let json = archive::read_cards_json(bytes, limits).map_err(Error::ImportRefused)?;
        let document = validate::parse(&json, limits).map_err(Error::ImportRefused)?;
        let tx = self.conn.unchecked_transaction()?;
        let summary = classify(&tx, &document)?.summary;
        // Read only: ending the transaction cannot lose anything.
        tx.finish()?;
        Ok(ImportPlan { document, summary })
    }

    /// Adds the plan's new decks, cards and review events to the database.
    ///
    /// Nothing is written if the plan has conflicts, or if every row is
    /// already present (then no backup is made either, so importing a file
    /// twice changes nothing). Otherwise the database is backed up first
    /// (no backup, no import), then, in one transaction, the file is
    /// compared again (refused if the database changed since the plan),
    /// the rows are inserted, and the schedule cache of every card that
    /// got reviews is rebuilt from its log.
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
        let summary = &plan.summary;
        if !summary.conflicts.is_empty() {
            return Err(Error::ImportConflicts(summary.conflicts.len()));
        }
        if summary.is_noop() {
            return Ok(ImportReport {
                decks: 0,
                cards: 0,
                review_events: 0,
                backup: None,
            });
        }

        let backup_dir = self.backup_dir.clone().ok_or(Error::NoBackupDir)?;
        let backup = create_backup(&self.conn, &backup_dir, now)?;
        tracing::info!(path = %backup.display(), "database backed up before an import");
        if let Err(err) = prune_backups(&backup_dir, KEEP_BACKUPS, Some(&backup)) {
            // As at startup: a failed prune only leaves extra backups.
            tracing::warn!("pruning old backups failed: {err}");
        }

        let doc = &plan.document;
        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let rows = classify(&tx, doc)?;
        // Rows are never removed, so between plan and now a row can only
        // go from new to present or to a conflict, or from present to a
        // conflict. Each of those changes the summary, so an equal summary
        // means the same rows are new. (A plan applied to another database,
        // after a restore, could match by counts alone; the rows written
        // are still exactly the new ones, as they come from this
        // comparison, not from the plan.)
        if rows.summary != *summary {
            return Err(Error::ImportPlanStale);
        }
        let reviewed = insert_rows(&tx, doc, &rows)?;
        rebuild_cards(&tx, &self.scheduler, reviewed)?;
        tx.commit()?;
        tracing::info!(
            decks = summary.decks.new,
            cards = summary.cards.new,
            review_events = summary.review_events.new,
            "import applied"
        );
        Ok(ImportReport {
            decks: summary.decks.new,
            cards: summary.cards.new,
            review_events: summary.review_events.new,
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
    let file = File::open(path).map_err(io_err)?;
    let too_large = Error::ImportRefused(ImportProblem::FileTooLarge { limit });
    if file.metadata().map_err(io_err)?.len() > limit {
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

/// How a document compares with the database: the summary, and the
/// positions of the rows to add.
struct Classified {
    summary: ImportSummary,
    new_decks: Vec<usize>,
    new_cards: Vec<usize>,
    new_events: Vec<usize>,
}

/// Same id and same content: present; unknown id: new; same id and
/// different content: a conflict. Then live deck names against the
/// database's other live decks.
fn classify(conn: &Connection, doc: &Document) -> Result<Classified> {
    let mut summary = ImportSummary::default();
    let mut new_decks = Vec::new();
    let mut new_cards = Vec::new();
    let mut new_events = Vec::new();

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
        match stored {
            None => {
                summary.decks.new += 1;
                new_decks.push(i);
            }
            Some(s) if s == *d => summary.decks.present += 1,
            Some(_) => summary.conflicts.push(Conflict::DeckDiffers(d.id.clone())),
        }
    }

    let mut stmt = conn.prepare(
        "SELECT id, deck_id, front_md, back_md, created_at, updated_at, deleted_at
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
                    deleted_at: r.get(6)?,
                })
            })
            .optional()?;
        match stored {
            None => {
                summary.cards.new += 1;
                new_cards.push(i);
            }
            Some(s) if s == *c => summary.cards.present += 1,
            Some(_) => summary.conflicts.push(Conflict::CardDiffers(c.id.clone())),
        }
    }

    let mut stmt = conn.prepare(
        "SELECT id, card_id, reviewed_at, grade, duration_ms FROM review_events WHERE id = ?1",
    )?;
    for (i, e) in doc.review_events.iter().enumerate() {
        let stored = stmt
            .query_row([&e.id], |r| {
                Ok(EventRecord {
                    id: r.get(0)?,
                    card_id: r.get(1)?,
                    reviewed_at: r.get(2)?,
                    grade: r.get(3)?,
                    duration_ms: r.get(4)?,
                })
            })
            .optional()?;
        match stored {
            None => {
                summary.review_events.new += 1;
                new_events.push(i);
            }
            Some(s) if s == *e => summary.review_events.present += 1,
            Some(_) => summary.conflicts.push(Conflict::EventDiffers(e.id.clone())),
        }
    }

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

    Ok(Classified {
        summary,
        new_decks,
        new_cards,
        new_events,
    })
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
        "INSERT INTO cards (id, deck_id, front_md, back_md, created_at, updated_at, deleted_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
    )?;
    for c in rows.new_cards.iter().map(|&i| &doc.cards[i]) {
        card.execute(params![
            c.id,
            c.deck_id,
            c.front_md,
            c.back_md,
            c.created_at,
            c.updated_at,
            c.deleted_at
        ])?;
    }
    let mut event = conn.prepare(
        "INSERT INTO review_events (id, card_id, reviewed_at, grade, duration_ms)
         VALUES (?1, ?2, ?3, ?4, ?5)",
    )?;
    let mut reviewed = BTreeSet::new();
    for e in rows.new_events.iter().map(|&i| &doc.review_events[i]) {
        event.execute(params![
            e.id,
            e.card_id,
            e.reviewed_at,
            e.grade,
            e.duration_ms
        ])?;
        reviewed.insert(e.card_id.as_str());
    }
    Ok(reviewed)
}
