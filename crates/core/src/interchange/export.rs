//! Exporting the whole database to a `.rappel` file.

use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};

use rusqlite::Connection;

use super::ExportSummary;
use super::archive::write_archive;
use super::format::{
    CardRecord, CardTagRecord, DeckRecord, Document, EventRecord, FORMAT_VERSION, SettingsRecord,
    TagRecord,
};
use crate::error::{Error, Result};
use crate::storage::{Store, stored_settings};
use crate::time::now_ms;

impl Store {
    /// Writes every deck, card, tag, card tag and log entry, deleted ones
    /// included, and the stored settings, to the `.rappel` file `path`,
    /// replacing it if it exists.
    ///
    /// The file is written next to `path` with mode `0600`, synced, and
    /// only then renamed, so `path` is never a partial export. The same
    /// database exported at the same time gives the same bytes (ADR 0017).
    ///
    /// # Errors
    /// SQLite and IO errors, or [`Error::ExportFailed`]. On error `path`
    /// is left as it was.
    pub fn export_to(&self, path: &Path) -> Result<ExportSummary> {
        let (bytes, summary) = self.export_bytes_at(now_ms())?;
        write_file_atomically(path, &bytes)?;
        Ok(summary)
    }

    /// The archive bytes of an export made at `now` (UTC milliseconds).
    pub(crate) fn export_bytes_at(&self, now: i64) -> Result<(Vec<u8>, ExportSummary)> {
        let document = read_document(&self.conn, now)?;
        let summary = ExportSummary {
            decks: document.decks.len(),
            cards: document.cards.len(),
            tags: document.tags.len(),
            review_events: document.review_events.len(),
        };
        let mut json =
            serde_json::to_vec_pretty(&document).map_err(|e| Error::ExportFailed(Box::new(e)))?;
        json.push(b'\n');
        let bytes = write_archive(&json).map_err(|e| Error::ExportFailed(Box::new(e)))?;
        Ok((bytes, summary))
    }
}

/// Every row and the stored settings, read in one transaction so the
/// tables agree, each table sorted by id (`card_tags` by card, then tag).
fn read_document(conn: &Connection, now: i64) -> Result<Document> {
    let tx = conn.unchecked_transaction()?;
    let decks = tx
        .prepare("SELECT id, name, created_at, updated_at, deleted_at FROM decks ORDER BY id")?
        .query_map([], |r| {
            Ok(DeckRecord {
                id: r.get(0)?,
                name: r.get(1)?,
                created_at: r.get(2)?,
                updated_at: r.get(3)?,
                deleted_at: r.get(4)?,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let cards = tx
        .prepare(
            "SELECT id, deck_id, front_md, back_md, created_at, updated_at, suspended, deleted_at
             FROM cards ORDER BY id",
        )?
        .query_map([], |r| {
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
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let tags = tx
        .prepare("SELECT id, name FROM tags ORDER BY id")?
        .query_map([], |r| {
            Ok(TagRecord {
                id: r.get(0)?,
                name: r.get(1)?,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let card_tags = tx
        .prepare("SELECT card_id, tag_id FROM card_tags ORDER BY card_id, tag_id")?
        .query_map([], |r| {
            Ok(CardTagRecord {
                card_id: r.get(0)?,
                tag_id: r.get(1)?,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let review_events = tx
        .prepare(
            "SELECT id, card_id, kind, reviewed_at, grade, duration_ms, undoes
             FROM review_events ORDER BY id",
        )?
        .query_map([], |r| {
            Ok(EventRecord {
                id: r.get(0)?,
                card_id: r.get(1)?,
                kind: r.get(2)?,
                reviewed_at: r.get(3)?,
                grade: r.get(4)?,
                duration_ms: r.get(5)?,
                undoes: r.get(6)?,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let stored = stored_settings(&tx)?;
    let settings = SettingsRecord {
        desired_retention: stored.desired_retention,
        max_new_per_day: stored.max_new_per_day,
        max_reviews_per_day: stored.max_reviews_per_day,
        day_start_hour: stored.day_start_hour,
    };
    // Read only: ending the transaction cannot lose anything.
    tx.finish()?;
    Ok(Document {
        format_version: FORMAT_VERSION,
        exported_at: now,
        decks,
        cards,
        tags,
        card_tags,
        review_events,
        settings,
        media: Vec::new(),
    })
}

/// Writes `bytes` to `<path>.partial` (mode `0600`), syncs it, renames it
/// to `path` and syncs the directory. The partial file is removed on
/// error.
fn write_file_atomically(path: &Path, bytes: &[u8]) -> Result<()> {
    let io_err = |p: &Path| {
        let path = p.to_path_buf();
        move |source| Error::Io { path, source }
    };
    let partial = partial_path(path);
    // A leftover from a crashed export is removed, and the file is then
    // created anew: an existing file would keep its mode, and a symlink
    // there would be followed.
    match fs::remove_file(&partial) {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => return Err(io_err(&partial)(e)),
    }
    let result = (|| {
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&partial)
            .map_err(io_err(&partial))?;
        file.write_all(bytes).map_err(io_err(&partial))?;
        file.sync_all().map_err(io_err(&partial))?;
        fs::rename(&partial, path).map_err(io_err(path))?;
        if let Some(dir) = path.parent().filter(|d| !d.as_os_str().is_empty()) {
            File::open(dir)
                .and_then(|d| d.sync_all())
                .map_err(io_err(dir))?;
        }
        Ok(())
    })();
    if result.is_err() {
        // Best effort: the error being returned matters more.
        let _ = fs::remove_file(&partial);
    }
    result
}

fn partial_path(path: &Path) -> PathBuf {
    let mut name = path.as_os_str().to_owned();
    name.push(".partial");
    PathBuf::from(name)
}
