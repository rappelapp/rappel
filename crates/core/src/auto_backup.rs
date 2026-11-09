//! Automatic backups to a folder the user chose (#127, ADR 0080).
//!
//! When Rappel closes, if the collection changed since the last automatic
//! backup and none was made today, it writes a normal export (format
//! version 2, with media) named `Rappel backup YYYY-MM-DD.rappel` into the
//! folder, then keeps the [`KEEP`] newest files whose names match that
//! pattern exactly and deletes the older ones. It never touches anything
//! else.
//!
//! Everything in the folder is done relative to one open handle on it
//! (`openat`, `renameat2`, `unlinkat`), so a folder renamed, removed or
//! swapped for a link while the backup runs never sends a write or a
//! deletion somewhere else. The folder itself is opened without following
//! a final symbolic link: the app stores the folder with its links
//! resolved when it is chosen, so a link found there later was put in its
//! place. The export is written to a hidden file with a random name that
//! never matches the pattern, synced, and only then renamed without
//! replacing anything, so a half-written backup never has a backup's name.
//! A file is deleted only if it is a regular file (checked without
//! following links, just before) whose name matches the pattern;
//! `unlinkat` never follows a link anyway, so nothing outside the folder
//! can be deleted.
//!
//! The decisions (the name, which files go, whether a backup is due) are
//! pure functions, tested here. The app keeps the folder and the last
//! backup in a key file in the data directory and runs [`run`] on a worker
//! thread when the window closes.

use std::fmt;
use std::fs::File;
use std::io;
use std::os::fd::OwnedFd;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

use rustix::fs::{AtFlags, Dir, FileType, Mode, OFlags, RenameFlags};
use rustix::io::Errno;

use crate::error::Error;
use crate::interchange::{ExportOptions, ExportSummary};
use crate::storage::Store;
use crate::time::{UtcParts, utc_ms, utc_parts};

/// How many automatic backups the folder keeps.
pub const KEEP: usize = 14;

/// What every automatic backup's name starts with.
const PREFIX: &str = "Rappel backup ";
/// What every automatic backup's name ends with (`interchange::FILE_EXTENSION`).
const SUFFIX: &str = ".rappel";
/// The most two local dates of one instant differ by: UTC+14 and UTC−12
/// are 26 hours apart, so 23:30 on one side is 01:30 two dates later on
/// the other. A last backup dated further ahead of today was made with a
/// wrong clock.
pub const MAX_ZONE_SPREAD_DAYS: i64 = 2;

/// The start of the hidden file a backup is written to before it is
/// renamed. A leading dot, so it never matches [`PREFIX`].
const PARTIAL_PREFIX: &str = ".Rappel backup ";
const PARTIAL_SUFFIX: &str = ".partial";
/// A backup is the user's data: only the user reads it, like the database.
const FILE_MODE: u32 = 0o600;

/// A calendar date in the user's time zone, as a backup's name has it.
/// Only years 1 to 9999, so the name always has four digits for the year.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct LocalDate {
    year: u16,
    month: u8,
    day: u8,
}

impl LocalDate {
    /// Days since 1970-01-01.
    fn days(self) -> i64 {
        crate::time::days_from_civil(
            i64::from(self.year),
            i64::from(self.month),
            i64::from(self.day),
        )
    }

    /// The date `year`-`month`-`day`, if it exists and the year is 1 to
    /// 9999.
    #[must_use]
    pub fn new(year: i32, month: i32, day: i32) -> Option<Self> {
        let date = Self {
            year: u16::try_from(year)
                .ok()
                .filter(|y| (1..=9999).contains(y))?,
            month: u8::try_from(month).ok()?,
            day: u8::try_from(day).ok()?,
        };
        // `utc_ms` refuses a day that does not exist (31 April, 29 February
        // of a common year).
        utc_ms(UtcParts {
            year: i64::from(date.year),
            month: i64::from(date.month),
            day: i64::from(date.day),
            hour: 0,
            minute: 0,
            second: 0,
        })?;
        Some(date)
    }

    /// The local date at `utc_ms` (UTC milliseconds) for a time zone
    /// `utc_offset_seconds` ahead of UTC (negative west of Greenwich), as
    /// the clock on the wall shows it then.
    #[must_use]
    pub fn from_utc_ms(utc_ms: i64, utc_offset_seconds: i32) -> Option<Self> {
        let local = utc_ms.checked_add(i64::from(utc_offset_seconds).checked_mul(1000)?)?;
        let parts = utc_parts(local);
        Self::new(
            i32::try_from(parts.year).ok()?,
            i32::try_from(parts.month).ok()?,
            i32::try_from(parts.day).ok()?,
        )
    }

    /// Reads `YYYY-MM-DD` exactly: ten ASCII characters, digits and two
    /// hyphens, naming a date that exists.
    #[must_use]
    pub fn parse(text: &str) -> Option<Self> {
        let bytes = text.as_bytes();
        let shape = bytes.len() == 10
            && bytes.iter().enumerate().all(|(i, b)| match i {
                4 | 7 => *b == b'-',
                _ => b.is_ascii_digit(),
            });
        if !shape {
            return None;
        }
        // Only ASCII digits now, so these parse.
        let number = |range: std::ops::Range<usize>| text.get(range)?.parse::<i32>().ok();
        Self::new(number(0..4)?, number(5..7)?, number(8..10)?)
    }
}

impl fmt::Display for LocalDate {
    /// `YYYY-MM-DD`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:04}-{:02}-{:02}", self.year, self.month, self.day)
    }
}

/// The name of the backup made on `date`: `Rappel backup 2026-11-09.rappel`.
#[must_use]
pub fn file_name(date: LocalDate) -> String {
    format!("{PREFIX}{date}{SUFFIX}")
}

/// The date in `name` if it is exactly the name of an automatic backup
/// ([`file_name`]); `None` for every other name, so no other file is ever
/// deleted.
#[must_use]
pub fn parse_file_name(name: &str) -> Option<LocalDate> {
    LocalDate::parse(name.strip_prefix(PREFIX)?.strip_suffix(SUFFIX)?)
}

/// Which of `found` (the backups in the folder, each a name and the date in
/// it) go: all but the [`KEEP`] newest by the date in their names. `keep`,
/// the backup just written, always stays, even if a clock that ran ahead
/// left backups dated later. The date in the name, not the file's time, so
/// a copy or a sync that touches the files changes nothing.
#[must_use]
pub fn to_delete(found: &[(String, LocalDate)], keep: &str) -> Vec<String> {
    let mut others: Vec<&(String, LocalDate)> =
        found.iter().filter(|(name, _)| name != keep).collect();
    // Newest first; a name holds one date, so no two share one, but the
    // name breaks a tie all the same.
    others.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| b.0.cmp(&a.0)));
    let slots = if found.iter().any(|(name, _)| name == keep) {
        KEEP - 1
    } else {
        KEEP
    };
    others
        .into_iter()
        .skip(slots)
        .map(|(name, _)| name.clone())
        .collect()
}

/// The last automatic backup, as the app keeps it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LastBackup {
    /// The local date it was made (the one in its name).
    pub date: LocalDate,
    /// The folder it went to.
    pub folder: PathBuf,
    /// What it held ([`Store`]'s collection fingerprint), or `None` if not
    /// known (the folder already had a backup of that day): then the next
    /// one is made whatever changed.
    pub fingerprint: Option<String>,
}

/// Whether a backup to `folder` may be due on `today`: none was made today
/// to this folder. A last backup dated up to [`MAX_ZONE_SPREAD_DAYS`] after
/// `today` (the user flew west) counts as today's, so travelling never
/// makes two backups with one date. One dated further ahead was made with
/// a clock set wrong; it does not stop backups until that date. A new
/// folder gets one at once.
#[must_use]
pub fn due(last: Option<&LastBackup>, folder: &Path, today: LocalDate) -> bool {
    match last {
        Some(last) if last.folder == folder => {
            last.date < today || last.date.days() - today.days() > MAX_ZONE_SPREAD_DAYS
        }
        _ => true,
    }
}

/// Whether the collection, whose fingerprint is now `fingerprint`, changed
/// since the last backup to `folder`. Unknown counts as changed.
#[must_use]
pub fn changed(last: Option<&LastBackup>, folder: &Path, fingerprint: &str) -> bool {
    match last {
        Some(last) if last.folder == folder => last.fingerprint.as_deref() != Some(fingerprint),
        _ => true,
    }
}

/// What [`run`] did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    /// Nothing changed since the last backup: nothing was written.
    Unchanged,
    /// The folder already has a backup named for today: it is kept as it
    /// is, never replaced.
    AlreadyThere,
    /// A backup was written.
    Written {
        /// Its name in the folder.
        file_name: String,
        /// What it holds, for the next [`changed`].
        fingerprint: String,
        /// What it holds, counted.
        summary: ExportSummary,
        /// How many older backups were deleted.
        removed: usize,
    },
}

/// Why a backup failed, for the message the app shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FailureKind {
    /// The folder is not there (removed, or a disk not plugged in).
    FolderMissing,
    /// Something other than a folder has its name.
    NotAFolder,
    /// A symbolic link has its name (see the module docs).
    FolderIsLink,
    /// The disk or the user's quota is full.
    NoSpace,
    /// The folder is on a read-only disk.
    ReadOnly,
    /// The folder may not be written to.
    NotAllowed,
    /// The backup would be larger than an import reads.
    TooLarge,
    /// Something that is not a regular file has today's backup's name.
    NameTaken,
    /// The app's time cap stopped it; its partial file was removed.
    Cancelled,
    /// Anything else (the database could not be read, a disk error).
    Other,
}

/// A backup that failed: why, and the error in English for the log.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Failure {
    /// Why, for the message the app shows.
    pub kind: FailureKind,
    /// The error in English, for the log and an unexpected error's toast.
    pub detail: String,
}

impl Failure {
    fn new(kind: FailureKind, detail: impl fmt::Display) -> Self {
        Self {
            kind,
            detail: detail.to_string(),
        }
    }

    /// A failure while opening the folder.
    fn opening_folder(err: Errno) -> Self {
        let kind = match err {
            Errno::LOOP => FailureKind::FolderIsLink,
            Errno::NOTDIR => FailureKind::NotAFolder,
            other => kind_of(&io::Error::from(other)),
        };
        Self::new(kind, format!("opening the backup folder: {err}"))
    }

    /// A failure while writing or renaming the backup.
    fn io(what: &str, err: &io::Error) -> Self {
        Self::new(kind_of(err), format!("{what}: {err}"))
    }

    /// A failure of core: an IO error says why only when it is about a file
    /// in the backup `folder` (writing the backup). Anything else, a media
    /// file that went missing or the database, is "other" with its error,
    /// so the toast never blames the folder for it.
    fn core(err: &Error, folder: &Path) -> Self {
        match err {
            Error::ExportTooLarge { .. } => Self::new(FailureKind::TooLarge, err),
            Error::Io { source, path } if path.starts_with(folder) => {
                Self::new(kind_of(source), err)
            }
            _ => Self::new(FailureKind::Other, err),
        }
    }
}

impl fmt::Display for Failure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.detail)
    }
}

/// Why `err` happened, as a [`FailureKind`]. A folder removed while the
/// backup runs gives `ENOENT` on the open handle, so "not found" is always
/// the folder.
#[must_use]
pub fn kind_of(err: &io::Error) -> FailureKind {
    match err.kind() {
        io::ErrorKind::NotFound => FailureKind::FolderMissing,
        io::ErrorKind::NotADirectory => FailureKind::NotAFolder,
        io::ErrorKind::StorageFull | io::ErrorKind::QuotaExceeded => FailureKind::NoSpace,
        io::ErrorKind::ReadOnlyFilesystem => FailureKind::ReadOnly,
        io::ErrorKind::PermissionDenied => FailureKind::NotAllowed,
        _ if err.raw_os_error() == Some(Errno::LOOP.raw_os_error()) => FailureKind::FolderIsLink,
        _ => FailureKind::Other,
    }
}

/// Makes the automatic backup of the database in `data_dir` into `folder`
/// for `today`, if the collection changed since `last` (the caller checked
/// [`due`] already); then deletes the backups beyond the [`KEEP`] newest.
/// Blocking: run it on a worker thread. It opens its own connection, reads
/// the database and never writes to it. Setting `cancel` stops the export
/// at its next write; the partial file is then removed.
///
/// A deletion that fails is logged and does not fail the backup.
///
/// # Errors
/// A [`Failure`] if the folder cannot be opened, the database cannot be
/// read, or the backup cannot be written; then nothing with a backup's name
/// was written, and the hidden partial file is removed if it can be.
pub fn run(
    data_dir: &Path,
    folder: &Path,
    today: LocalDate,
    last: Option<&LastBackup>,
    cancel: &AtomicBool,
) -> Result<Outcome, Failure> {
    // The collection first: nothing changed means nothing to do, even with
    // the folder's disk unplugged.
    let store = Store::open_concurrent(data_dir).map_err(|err| Failure::core(&err, folder))?;
    let fingerprint = store
        .collection_fingerprint()
        .map_err(|err| Failure::core(&err, folder))?;
    if !changed(last, folder, &fingerprint) {
        return Ok(Outcome::Unchanged);
    }
    let dir = open_folder(folder)?;
    let name = file_name(today);
    match rustix::fs::statat(&dir, name.as_str(), AtFlags::SYMLINK_NOFOLLOW) {
        // Whatever it is, it stays: a regular file of that name is today's
        // backup already (made before the app's record of it was saved);
        // anything else is not Rappel's, and the user is told.
        Ok(_) => return already_there(&dir, &name),
        Err(Errno::NOENT) => {}
        Err(err) => return Err(Failure::io("looking for today's backup", &err.into())),
    }
    let summary = match write_backup(&dir, folder, &name, &store, cancel) {
        Ok(summary) => summary,
        Err(Written::Exists) => return already_there(&dir, &name),
        Err(Written::Failed(failure)) => return Err(failure),
    };
    drop(store);
    let removed = prune(&dir, &name);
    Ok(Outcome::Written {
        file_name: name,
        fingerprint,
        summary,
        removed,
    })
}

/// Today's `name` is taken: by today's backup (a regular file), or by
/// something else, which is a failure to report.
fn already_there(dir: &OwnedFd, name: &str) -> Result<Outcome, Failure> {
    if is_regular_file(dir, name) {
        Ok(Outcome::AlreadyThere)
    } else {
        Err(Failure::new(
            FailureKind::NameTaken,
            "something that is not a backup has today's backup's name",
        ))
    }
}

/// Opens `folder` as a directory, refusing a symbolic link in its place
/// (`O_NOFOLLOW`) and anything that is not a folder (`O_DIRECTORY`). With
/// both flags Linux answers `ENOTDIR` for a link, the same as for a file
/// (`ELOOP` only without `O_DIRECTORY`; open(2)), so the path is looked at
/// again, without following it, to say which.
fn open_folder(folder: &Path) -> Result<OwnedFd, Failure> {
    rustix::fs::open(
        folder,
        OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
        Mode::empty(),
    )
    .map_err(|err| {
        let is_link = std::fs::symlink_metadata(folder).is_ok_and(|m| m.file_type().is_symlink());
        if err == Errno::NOTDIR && is_link {
            Failure::opening_folder(Errno::LOOP)
        } else {
            Failure::opening_folder(err)
        }
    })
}

/// Why [`write_backup`] did not write.
enum Written {
    /// Something named for today appeared meanwhile; it is kept.
    Exists,
    Failed(Failure),
}

/// Writes the export to a hidden partial file in `dir`, syncs it, renames
/// it to `name` without replacing anything, and syncs the folder. On error
/// the partial file, which this call created, is removed.
fn write_backup(
    dir: &OwnedFd,
    folder: &Path,
    name: &str,
    store: &Store,
    cancel: &AtomicBool,
) -> Result<ExportSummary, Written> {
    let partial = partial_name(name).map_err(|err| Written::Failed(Failure::core(&err, folder)))?;
    // A new file (`O_EXCL`, so never one that was there, and never through
    // a link), only for the user.
    let fd = rustix::fs::openat(
        dir,
        partial.as_str(),
        OFlags::WRONLY | OFlags::CREATE | OFlags::EXCL | OFlags::NOFOLLOW | OFlags::CLOEXEC,
        Mode::from_raw_mode(FILE_MODE),
    )
    .map_err(|err| Written::Failed(Failure::io("creating the backup", &err.into())))?;
    let mut file = File::from(fd);
    let result = (|| {
        let summary = store
            .export_into(
                &mut file,
                &folder.join(&partial),
                ExportOptions::default(),
                crate::interchange::MAX_ARCHIVE_BYTES,
                cancel,
            )
            .map_err(|err| {
                Written::Failed(if cancel.load(Ordering::Relaxed) {
                    Failure::new(FailureKind::Cancelled, "stopped by the time limit")
                } else {
                    Failure::core(&err, folder)
                })
            })?;
        file.sync_all()
            .map_err(|err| Written::Failed(Failure::io("saving the backup", &err)))?;
        rename_no_replace(dir, &partial, name)?;
        // The rename itself is only safe on disk once the folder is synced.
        // A file system that cannot sync a folder (`EINVAL`, fsync(2)) has
        // the backup all the same.
        match rustix::fs::fsync(dir) {
            Ok(()) | Err(Errno::INVAL | Errno::NOTSUP) => {}
            Err(err) => {
                return Err(Written::Failed(Failure::io(
                    "saving the backup",
                    &err.into(),
                )));
            }
        }
        Ok(summary)
    })();
    drop(file);
    if result.is_err() {
        // Best effort: the error being returned matters more. Only the
        // file this call created (`O_EXCL`), by its random name.
        let _ = rustix::fs::unlinkat(dir, partial.as_str(), AtFlags::empty());
    }
    result
}

/// `.Rappel backup 2026-11-09.rappel.<16 hex digits>.partial`: hidden,
/// random, and never a backup's name.
fn partial_name(name: &str) -> crate::Result<String> {
    let mut random = [0_u8; 8];
    getrandom::fill(&mut random).map_err(|err| Error::Random(Box::new(err)))?;
    let rest = name.strip_prefix(PREFIX).unwrap_or(name);
    Ok(format!(
        "{PARTIAL_PREFIX}{rest}.{}{PARTIAL_SUFFIX}",
        crate::media::hex(&random)
    ))
}

/// Renames `from` to `to` in `dir`, never over an existing file:
/// `renameat2` with `RENAME_NOREPLACE`. A file system without that flag
/// (`EINVAL`) gets a check that `to` is free and a plain rename; only a
/// file made under that name in between could then be replaced.
fn rename_no_replace(dir: &OwnedFd, from: &str, to: &str) -> Result<(), Written> {
    let failed = |err: Errno| Written::Failed(Failure::io("naming the backup", &err.into()));
    match rustix::fs::renameat_with(dir, from, dir, to, RenameFlags::NOREPLACE) {
        Ok(()) => Ok(()),
        Err(Errno::EXIST) => Err(Written::Exists),
        Err(Errno::INVAL | Errno::NOSYS | Errno::NOTSUP) => {
            match rustix::fs::statat(dir, to, AtFlags::SYMLINK_NOFOLLOW) {
                Ok(_) => return Err(Written::Exists),
                Err(Errno::NOENT) => {}
                Err(err) => return Err(failed(err)),
            }
            rustix::fs::renameat(dir, from, dir, to).map_err(failed)
        }
        Err(err) => Err(failed(err)),
    }
}

/// The backups in `dir`: regular files (not links, not folders; checked
/// with `fstatat` without following links, since a directory entry's type
/// may be unknown) whose names match the pattern exactly.
fn backups_in(dir: &OwnedFd) -> io::Result<Vec<(String, LocalDate)>> {
    let mut found = Vec::new();
    for entry in Dir::read_from(dir)? {
        let entry = entry?;
        let Some(name) = entry.file_name().to_str().ok() else {
            continue;
        };
        let Some(date) = parse_file_name(name) else {
            continue;
        };
        if is_regular_file(dir, name) {
            found.push((name.to_owned(), date));
        }
    }
    Ok(found)
}

/// Whether `name` in `dir` is a regular file, without following a link.
fn is_regular_file(dir: &OwnedFd, name: &str) -> bool {
    rustix::fs::statat(dir, name, AtFlags::SYMLINK_NOFOLLOW)
        .is_ok_and(|stat| FileType::from_raw_mode(stat.st_mode) == FileType::RegularFile)
}

/// Deletes the backups in `dir` beyond the [`KEEP`] newest, never `keep`
/// (the one just written); how many went. Each is checked again just
/// before it goes. Failures are logged.
fn prune(dir: &OwnedFd, keep: &str) -> usize {
    let found = match backups_in(dir) {
        Ok(found) => found,
        Err(err) => {
            tracing::warn!(
                "old automatic backups not removed: the folder could not be read: {err}"
            );
            return 0;
        }
    };
    let mut removed = 0;
    for name in to_delete(&found, keep) {
        // Checked again: a file swapped for a link since the listing stays.
        // A swap in the instant between this check and `unlinkat` would
        // remove that link (never what it points to: `unlinkat` does not
        // follow links); std and Linux offer no unlink-if-regular.
        if !is_regular_file(dir, &name) {
            continue;
        }
        match rustix::fs::unlinkat(dir, name.as_str(), AtFlags::empty()) {
            Ok(()) => removed += 1,
            Err(err) => tracing::warn!("an old automatic backup could not be removed: {err}"),
        }
    }
    removed
}

#[cfg(test)]
mod tests;
