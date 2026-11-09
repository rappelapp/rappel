//! Exporting the whole database to a `.rappel` file.

use std::fs::{self, File, OpenOptions};
use std::io::{self, BufWriter, Seek, Write};
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};

use rusqlite::Connection;
use sha2::{Digest, Sha256};

use super::archive::ArchiveWriter;
use super::format::{
    CardRecord, CardTagRecord, DeckRecord, Document, EventRecord, FORMAT_VERSION, MediaRecord,
    SettingsRecord, TagRecord,
};
use super::{ExportOptions, ExportSummary, Limits, MAX_ARCHIVE_BYTES};
use crate::error::{Error, Result};
use crate::media::{CopyFailure, MediaKind, MediaRef, copy_capped, parse_hex};
use crate::storage::{Store, stored_settings};
use crate::time::now_ms;

impl Store {
    /// Writes every deck, card, tag, card tag and log entry, deleted ones
    /// included, the stored settings and every media file, to the
    /// `.rappel` file `path`, replacing it if it exists.
    ///
    /// The file is written next to `path` with mode `0600`, synced, and
    /// only then renamed, so `path` is never a partial export. Media files
    /// are streamed from the media folder, never held in memory. A media
    /// file that is missing or damaged is left out with its record and
    /// counted in [`ExportSummary::missing_media`]. The same database
    /// exported at the same time gives the same bytes (ADR 0017, 0054).
    ///
    /// # Errors
    /// SQLite and IO errors, or [`Error::ExportFailed`]. On error `path`
    /// is left as it was.
    pub fn export_to(&self, path: &Path) -> Result<ExportSummary> {
        self.export_to_with(path, ExportOptions::default())
    }

    /// [`Self::export_to`] with `options`: without media, the file has no
    /// media entry and no media record (ADR 0071).
    ///
    /// # Errors
    /// Those of [`Self::export_to`], and [`Error::ExportTooLarge`] if the
    /// file came out larger than an import reads (what
    /// [`media_too_large_for_import`] cannot know beforehand: the size of
    /// `cards.json`); then `path` is left as it was.
    pub fn export_to_with(&self, path: &Path, options: ExportOptions) -> Result<ExportSummary> {
        self.export_to_limited(path, options, MAX_ARCHIVE_BYTES)
    }

    /// [`Self::export_to_with`] with the size an import reads as `limit`,
    /// for tests.
    pub(crate) fn export_to_limited(
        &self,
        path: &Path,
        options: ExportOptions,
        limit: u64,
    ) -> Result<ExportSummary> {
        write_file_atomically(path, |file| self.export_into(file, path, options, limit))
    }

    /// Writes the export to `file`, a new empty file opened for writing,
    /// without syncing it; `path` only names it in errors. The automatic
    /// backup (`auto_backup`, ADR 0080) writes into a file it opened in its
    /// folder itself.
    ///
    /// # Errors
    /// Those of [`Self::export_to_with`], [`Error::ExportTooLarge`] when the
    /// file came out larger than `limit`.
    pub(crate) fn export_into(
        &self,
        file: &mut File,
        path: &Path,
        options: ExportOptions,
        limit: u64,
    ) -> Result<ExportSummary> {
        let mut out = BufWriter::with_capacity(64 << 10, file);
        let summary = self.write_export(&mut out, now_ms(), options)?;
        out.flush().map_err(|source| Error::Io {
            path: path.to_path_buf(),
            source,
        })?;
        let bytes = out
            .get_ref()
            .metadata()
            .map_err(|source| Error::Io {
                path: path.to_path_buf(),
                source,
            })?
            .len();
        if bytes > limit {
            return Err(Error::ExportTooLarge { bytes, limit });
        }
        Ok(summary)
    }

    /// A fingerprint of everything an export holds but its date: the
    /// SHA-256, in hex, of `cards.json` as an export with media would write
    /// it with `exported_at` 0, every media record included. Two calls give
    /// the same text exactly when an export would hold the same decks,
    /// cards, tags, log, settings and media list (the export is
    /// deterministic, ADR 0017). The automatic backup compares it with the
    /// last backup's to know whether the collection changed (ADR 0080):
    /// unlike a timestamp, it sees every kind of change (tags, suspension,
    /// a move, archiving), and it needs no new column. The media files
    /// are not read: their records name them by content.
    ///
    /// # Errors
    /// SQLite errors.
    pub(crate) fn collection_fingerprint(&self) -> Result<String> {
        let (mut document, rows) = read_document(&self.conn, 0)?;
        document.media = rows.into_iter().map(|(record, _)| record).collect();
        let mut hasher = HashWriter(Sha256::new());
        serde_json::to_writer(&mut hasher, &document)
            .map_err(|e| Error::ExportFailed(Box::new(e)))?;
        Ok(crate::media::hex(&hasher.0.finalize()))
    }

    /// The media an export would hold, for [`media_too_large_for_import`]:
    /// how many files the database lists and their sizes together (the
    /// `bytes` column). One cheap query; no file is touched.
    ///
    /// # Errors
    /// SQLite errors.
    pub fn media_totals(&self) -> Result<MediaTotals> {
        let (bytes, files): (i64, i64) = self.conn.query_row(
            "SELECT COALESCE(SUM(bytes), 0), COUNT(*) FROM media",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )?;
        Ok(MediaTotals {
            bytes: u64::try_from(bytes).unwrap_or(0),
            files: usize::try_from(files).unwrap_or(0),
        })
    }

    /// The archive bytes of an export made at `now` (UTC milliseconds).
    #[cfg(test)]
    pub(crate) fn export_bytes_at(&self, now: i64) -> Result<(Vec<u8>, ExportSummary)> {
        self.export_bytes_with(now, ExportOptions::default())
    }

    /// [`Self::export_bytes_at`] with `options`.
    #[cfg(test)]
    pub(crate) fn export_bytes_with(
        &self,
        now: i64,
        options: ExportOptions,
    ) -> Result<(Vec<u8>, ExportSummary)> {
        let mut out = io::Cursor::new(Vec::new());
        let summary = self.write_export(&mut out, now, options)?;
        Ok((out.into_inner(), summary))
    }

    /// Writes the export made at `now` to `out`.
    fn write_export<W: Write + Seek>(
        &self,
        out: W,
        now: i64,
        options: ExportOptions,
    ) -> Result<ExportSummary> {
        let (mut document, rows) = read_document(&self.conn, now)?;
        // Asked to leave the media out: no record, no entry, and the rows
        // are counted as left out, not as missing.
        let media_left_out = if options.include_media { 0 } else { rows.len() };
        let rows = if options.include_media {
            rows
        } else {
            Vec::new()
        };
        // Only the files that are there and whole go in, with their
        // records, so the export stays importable.
        let mut files = Vec::with_capacity(rows.len());
        let mut missing_media = 0;
        for (record, media) in rows {
            match media.and_then(|m| self.media_file(&m, &record).map(|path| (m, path))) {
                Some((media, path)) => {
                    document.media.push(record);
                    files.push((media, path));
                }
                None => missing_media += 1,
            }
        }
        if missing_media > 0 {
            tracing::warn!(
                missing_media,
                "media files missing or damaged, left out of the export"
            );
        }
        let summary = ExportSummary {
            decks: document.decks.len(),
            cards: document.cards.len(),
            tags: document.tags.len(),
            review_events: document.review_events.len(),
            media: files.len(),
            media_left_out,
            missing_media,
        };
        let mut json =
            serde_json::to_vec_pretty(&document).map_err(|e| Error::ExportFailed(Box::new(e)))?;
        json.push(b'\n');
        drop(document);
        let failed = |e: zip::result::ZipError| Error::ExportFailed(Box::new(e));
        let mut archive = ArchiveWriter::new(out, &json).map_err(failed)?;
        drop(json);
        for (media, path) in &files {
            let entry = archive.start_media(&media.file_name()).map_err(failed)?;
            copy_media(path, media, entry)?;
        }
        archive.finish().map_err(failed)?;
        Ok(summary)
    }

    /// The path of `media`'s file if it is a regular file whose size, kind
    /// and SHA-256 are those recorded (read once to check); else `None`,
    /// and it is left out of the export.
    fn media_file(&self, media: &MediaRef, record: &MediaRecord) -> Option<PathBuf> {
        let path = self.media_path(media)?;
        let mut file = open_regular(&path).ok()?;
        let copied = copy_capped(&mut file, &mut io::sink()).ok()?;
        let whole =
            copied.media == *media && i64::try_from(copied.bytes).ok() == Some(record.bytes);
        whole.then_some(path)
    }
}

/// Opens `path` for reading if it is a regular file, never through a
/// symbolic link (the media folder holds only Rappel's own files).
fn open_regular(path: &Path) -> io::Result<File> {
    let meta = fs::symlink_metadata(path)?;
    if !meta.is_file() {
        return Err(io::Error::other("not a regular file"));
    }
    let file = File::open(path)?;
    if !file.metadata()?.is_file() {
        return Err(io::Error::other("not a regular file"));
    }
    Ok(file)
}

/// Streams the media file at `path` into the archive entry `out`, hashing
/// it again: a file that changed since it was checked fails the export
/// (the entry is written already, so it cannot be left out any more).
fn copy_media(path: &Path, media: &MediaRef, out: &mut impl Write) -> Result<()> {
    let io_err = |source| Error::Io {
        path: path.to_path_buf(),
        source,
    };
    let mut file = open_regular(path).map_err(io_err)?;
    match copy_capped(&mut file, out) {
        Ok(copied) if copied.media == *media => Ok(()),
        Ok(_) | Err(CopyFailure::Refused(_)) => Err(Error::ExportFailed(
            "a media file changed while it was exported".into(),
        )),
        Err(CopyFailure::Read(source) | CopyFailure::Write(source)) => Err(io_err(source)),
    }
}

/// Every row and the stored settings, read in one transaction so the
/// tables agree, each table sorted by id (`card_tags` by card, then tag),
/// and the media rows ([`media_rows`]). The document's `media` is empty:
/// the caller adds the files it finds.
fn read_document(conn: &Connection, now: i64) -> Result<(Document, Vec<MediaRow>)> {
    let tx = conn.unchecked_transaction()?;
    let decks = tx
        .prepare(
            "SELECT id, name, created_at, updated_at, archived_at, deleted_at
             FROM decks ORDER BY id",
        )?
        .query_map([], |r| {
            Ok(DeckRecord {
                id: r.get(0)?,
                name: r.get(1)?,
                created_at: r.get(2)?,
                updated_at: r.get(3)?,
                archived_at: r.get::<_, Option<i64>>(4)?.into(),
                deleted_at: r.get(5)?,
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
    let media = media_rows(&tx)?;
    let stored = stored_settings(&tx)?;
    let settings = SettingsRecord {
        desired_retention: stored.desired_retention,
        max_new_per_day: stored.max_new_per_day,
        max_reviews_per_day: stored.max_reviews_per_day,
        day_start_hour: stored.day_start_hour,
    };
    // Read only: ending the transaction cannot lose anything.
    tx.finish()?;
    Ok((
        Document {
            format_version: FORMAT_VERSION,
            exported_at: now,
            decks,
            cards,
            tags,
            card_tags,
            review_events,
            settings,
            media: Vec::new(),
        },
        media,
    ))
}

/// A media row and the file it names, if Rappel can name one.
type MediaRow = (MediaRecord, Option<MediaRef>);

/// The media rows, sorted by hash, each with its file: `None` for a row
/// whose hash or type Rappel cannot name (the schema's CHECKs keep them
/// out, but a damaged database may have one), which the export counts as
/// missing.
fn media_rows(conn: &Connection) -> Result<Vec<MediaRow>> {
    let rows = conn
        .prepare("SELECT hash, mime, bytes, created_at FROM media ORDER BY hash")?
        .query_map([], |r| {
            Ok(MediaRecord {
                hash: r.get(0)?,
                mime: r.get(1)?,
                bytes: r.get(2)?,
                created_at: r.get(3)?,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows
        .into_iter()
        .map(|record| {
            let media = parse_hex(record.hash.as_bytes())
                .zip(MediaKind::from_mime(&record.mime))
                .map(|(hash, kind)| MediaRef::new(hash, kind));
            (record, media)
        })
        .collect())
}

/// Writes `<path>.partial` (mode `0600`) with `write`, syncs it, renames
/// it to `path` and syncs the directory. The partial file is removed on
/// error.
fn write_file_atomically<T>(path: &Path, write: impl FnOnce(&mut File) -> Result<T>) -> Result<T> {
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
        let value = write(&mut file)?;
        file.sync_all().map_err(io_err(&partial))?;
        fs::rename(&partial, path).map_err(io_err(path))?;
        if let Some(dir) = path.parent().filter(|d| !d.as_os_str().is_empty()) {
            File::open(dir)
                .and_then(|d| d.sync_all())
                .map_err(io_err(dir))?;
        }
        Ok(value)
    })();
    if result.is_err() {
        // Best effort: the error being returned matters more.
        let _ = fs::remove_file(&partial);
    }
    result
}

/// Feeds what is written to a SHA-256.
struct HashWriter(Sha256);

impl Write for HashWriter {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.0.update(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

fn partial_path(path: &Path) -> PathBuf {
    let mut name = path.as_os_str().to_owned();
    name.push(".partial");
    PathBuf::from(name)
}

/// The media an export would hold (see [`Store::media_totals`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MediaTotals {
    /// The media files' sizes together, in bytes (the `bytes` column).
    pub bytes: u64,
    /// How many media files.
    pub files: usize,
}

/// Why an export with its media could not be imported again
/// ([`media_too_large_for_import`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MediaOverLimit {
    /// The archive would take at least `at_least` bytes
    /// (`export_size_at_least`), more than the 2 GiB an import reads
    /// (`MAX_ARCHIVE_BYTES`).
    Size {
        /// The least the archive would take.
        at_least: u64,
    },
    /// The archive would hold more entries than an import reads.
    Files {
        /// How many media files.
        files: usize,
    },
}

/// The least a zip entry takes besides its data in an archive Rappel
/// writes (no extra field, no comment, no data descriptor): the local
/// header (30 bytes) and the central record (46) with the name twice. The
/// shortest name is `media/` + 64 hex digits + `.` + a three-letter
/// extension (`MediaKind::extension`) = 74 bytes.
const MEDIA_ENTRY_OVERHEAD: u64 = 30 + 46 + 2 * 74;

/// The bytes of an archive besides its media and the data of
/// `cards.json`: the directory entry `media/` (30 + 46 + 2 × 6),
/// `cards.json`'s headers (30 + 46 + 2 × 10) and the end record (22).
const ARCHIVE_OVERHEAD: u64 = (30 + 46 + 12) + (30 + 46 + 20) + 22;

/// The least the archive of an export with `media` takes: the media, their
/// entries' headers and the archive's own, with `cards.json` taken as
/// empty. A lower bound, so that what it refuses is refused rightly: the
/// compressed size of `cards.json` cannot be known without writing it, and
/// any upper bound of it is far above the real size (Deflate shrinks the
/// repeated keys and ids many times), which would turn away exports that
/// import fine (ADR 0071).
#[must_use]
pub(crate) fn export_size_at_least(media: MediaTotals) -> u64 {
    let files = u64::try_from(media.files).unwrap_or(u64::MAX);
    media
        .bytes
        .saturating_add(files.saturating_mul(MEDIA_ENTRY_OVERHEAD))
        .saturating_add(ARCHIVE_OVERHEAD)
}

/// Why an import would refuse an export with `media`, for what leaving the
/// media out mends, or `None`: `export_size_at_least` past the 2 GiB an
/// import reads (`MAX_ARCHIVE_BYTES`), else more entries (the media files, `cards.json`
/// and `media/`) than an import reads. Both are lower bounds, so a `Some`
/// is always right for the media the database lists (a file missing from
/// the media folder is left out of the export, which is then smaller).
/// A `None` is not a promise: `cards.json` may push the file over, which
/// the export finds once it has written it ([`Error::ExportTooLarge`]).
/// The app asks before exporting (ADR 0071).
#[must_use]
pub fn media_too_large_for_import(media: MediaTotals) -> Option<MediaOverLimit> {
    let at_least = export_size_at_least(media);
    let entries = u64::try_from(media.files)
        .unwrap_or(u64::MAX)
        .saturating_add(2);
    if at_least > MAX_ARCHIVE_BYTES {
        Some(MediaOverLimit::Size { at_least })
    } else if entries > Limits::DEFAULT.entries {
        Some(MediaOverLimit::Files { files: media.files })
    } else {
        None
    }
}
