//! The automatic backup folder (#127, ADR 0080): its setting and the record
//! of the last backup in `<data dir>/auto-backup.ini`, the backup made when
//! the window closes, and the toast at the next start when it failed.
//!
//! A key file in the data directory, like `window-state.ini` (ADR 0039):
//! it follows `RAPPEL_DATA_DIR` and the debug data directory, needs no
//! schema or migration, and stays out of exports and database backups. The
//! folder belongs to this computer.
//!
//! The backup itself is `rappel_core::auto_backup::run`, on a worker
//! thread. The window closes at once; the application is held until the
//! backup ends, for at most [`TIME_CAP`], so a slow or stuck disk never
//! keeps a windowless process for long. A backup cut off, failed, or never
//! finished (the computer shut down) is reported by a toast at the next
//! start, and is tried again at the next close: nothing is recorded as
//! backed up until it is.

use std::cell::{Cell, RefCell};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use gtk::prelude::*;
use gtk::{gio, glib};
use rappel_core::auto_backup::{self, Failure, FailureKind, LastBackup, LocalDate, Outcome};

use crate::i18n::{fill, gettext, pgettext};
use crate::key_file::{self, Unread};
use crate::ui::toast::show_error;

/// The file name in the data directory.
const FILE_NAME: &str = "auto-backup.ini";
/// A file Rappel writes is well under this (a path, a date, a hash, a
/// short error); a larger one was not written by it, and is not read.
const MAX_FILE_BYTES: u64 = 16 * 1024;
/// The longest error kept for the next start's toast, in characters.
const MAX_DETAIL_CHARS: usize = 1000;

const SETTINGS: &str = "settings";
const FOLDER: &str = "folder";
const LAST: &str = "last-backup";
const DATE: &str = "date";
const FINGERPRINT: &str = "fingerprint";
const FAILURE: &str = "failure";
const REASON: &str = "reason";
const DETAIL: &str = "detail";
const RUNNING: &str = "running";

/// How long the application waits after the window closed before it
/// cancels the backup. A 2 GiB export (the most an import reads) reads
/// each media file twice and writes it once; on a USB 2 stick at 5 MB/s
/// that can take longer, and such a backup is cancelled and reported (its
/// partial file removed) rather than keep a windowless process for long.
pub const TIME_CAP: Duration = Duration::from_mins(5);

/// How long a cancelled backup gets to stop and remove its partial file
/// before the application lets go of it. Its next write fails at once, so
/// only a disk stuck inside one write needs this.
const CANCEL_GRACE: Duration = Duration::from_secs(30);

thread_local! {
    /// A backup of this process is running (on the main thread: only it
    /// starts and ends one).
    static IN_PROGRESS: Cell<bool> = const { Cell::new(false) };
}

/// Why the last automatic backup was not made.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reason {
    /// It failed, for this reason.
    Failed(FailureKind),
    /// It took longer than [`TIME_CAP`].
    TimedOut,
    /// It was still running when Rappel was stopped.
    Unfinished,
    /// The worker stopped unexpectedly (a panic).
    Stopped,
}

impl Reason {
    /// The word kept in the key file.
    fn code(self) -> &'static str {
        match self {
            Self::Failed(FailureKind::FolderMissing) => "folder-missing",
            Self::Failed(FailureKind::NotAFolder) => "not-a-folder",
            Self::Failed(FailureKind::FolderIsLink) => "folder-is-link",
            Self::Failed(FailureKind::NoSpace) => "no-space",
            Self::Failed(FailureKind::ReadOnly) => "read-only",
            Self::Failed(FailureKind::NotAllowed) => "not-allowed",
            Self::Failed(FailureKind::TooLarge) => "too-large",
            Self::Failed(FailureKind::NameTaken) => "name-taken",
            Self::Failed(FailureKind::Other) => "other",
            // Cancelled by the time cap: the same as timed out.
            Self::TimedOut | Self::Failed(FailureKind::Cancelled) => "timed-out",
            Self::Unfinished => "unfinished",
            Self::Stopped => "stopped",
        }
    }

    /// The reason a word in the key file names; an unknown word is
    /// "other", so a failure is never lost.
    fn from_code(code: &str) -> Self {
        [
            FailureKind::FolderMissing,
            FailureKind::NotAFolder,
            FailureKind::FolderIsLink,
            FailureKind::NoSpace,
            FailureKind::ReadOnly,
            FailureKind::NotAllowed,
            FailureKind::TooLarge,
            FailureKind::NameTaken,
        ]
        .map(Self::Failed)
        .into_iter()
        .chain([Self::TimedOut, Self::Unfinished, Self::Stopped])
        .find(|reason| reason.code() == code)
        .unwrap_or(Self::Failed(FailureKind::Other))
    }
}

/// What `auto-backup.ini` holds.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Record {
    /// The folder chosen in Preferences; `None`: off (the default).
    pub folder: Option<PathBuf>,
    /// The last backup made.
    pub last: Option<LastBackup>,
    /// The last backup that failed, to report at the next start, with the
    /// error in English.
    pub failure: Option<(Reason, String)>,
    /// A backup started on this date and has not ended (yet).
    pub running: Option<LocalDate>,
}

/// A stored folder if it can be one: an absolute path.
fn folder_of(text: Option<String>) -> Option<PathBuf> {
    text.map(PathBuf::from).filter(|path| path.is_absolute())
}

/// A stored fingerprint if it can be one: 64 lowercase hex digits.
fn fingerprint_of(text: Option<String>) -> Option<String> {
    text.filter(|t| t.len() == 64 && t.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f')))
}

/// Reads the record from a loaded key file. Every missing or bad value
/// is left out; this never fails.
fn from_key_file(file: &glib::KeyFile) -> Record {
    let string = |group: &str, key: &str| file.string(group, key).ok().map(String::from);
    let date = |group: &str| string(group, DATE).as_deref().and_then(LocalDate::parse);
    let last = date(LAST)
        .zip(folder_of(string(LAST, FOLDER)))
        .map(|(date, folder)| LastBackup {
            date,
            folder,
            fingerprint: fingerprint_of(string(LAST, FINGERPRINT)),
        });
    let failure = string(FAILURE, REASON).map(|code| {
        (
            Reason::from_code(&code),
            string(FAILURE, DETAIL).unwrap_or_default(),
        )
    });
    Record {
        folder: folder_of(string(SETTINGS, FOLDER)),
        last,
        failure,
        running: date(RUNNING),
    }
}

/// `path` as the key file keeps it, if it is UTF-8 (`GKeyFile` strings are).
fn path_text(path: &Path) -> Option<&str> {
    path.to_str()
}

/// The key file holding `record`.
fn to_key_file(record: &Record) -> glib::KeyFile {
    let file = glib::KeyFile::new();
    if let Some(folder) = record.folder.as_deref().and_then(path_text) {
        file.set_string(SETTINGS, FOLDER, folder);
    }
    if let Some(last) = &record.last
        && let Some(folder) = path_text(&last.folder)
    {
        file.set_string(LAST, DATE, &last.date.to_string());
        file.set_string(LAST, FOLDER, folder);
        if let Some(fingerprint) = &last.fingerprint {
            file.set_string(LAST, FINGERPRINT, fingerprint);
        }
    }
    if let Some((reason, detail)) = &record.failure {
        file.set_string(FAILURE, REASON, reason.code());
        let detail: String = detail.chars().take(MAX_DETAIL_CHARS).collect();
        file.set_string(FAILURE, DETAIL, &detail);
    }
    if let Some(date) = record.running {
        file.set_string(RUNNING, DATE, &date.to_string());
    }
    file
}

fn path(data_dir: &Path) -> PathBuf {
    data_dir.join(FILE_NAME)
}

/// The record in `data_dir`; empty (backups off) if there is none or it
/// cannot be read. Never fails, never blocks (`key_file::read_bounded`).
pub fn load(data_dir: &Path) -> Record {
    load_or_why(data_dir).unwrap_or_default()
}

/// The record in `data_dir` (empty if there is no file), or why a file
/// that is there could not be read: then backups are off, which the next
/// start says ([`report_at_start`]).
fn load_or_why(data_dir: &Path) -> Result<Record, String> {
    let read = match key_file::read_bounded(&path(data_dir), MAX_FILE_BYTES) {
        Ok(bytes) => key_file::parse(&bytes).map(|file| from_key_file(&file)),
        Err(Unread::Missing) => Ok(Record::default()),
        Err(Unread::Refused(why)) => Err(why),
    };
    if let Err(why) = &read {
        tracing::warn!("the automatic backup setting is not readable: {why}");
    }
    read
}

/// Whether `folder` is inside `data_dir` (both with their links resolved):
/// a backup there would be lost with the data it backs up, and could land
/// in the media or backups folders Rappel manages itself.
pub fn inside_data_dir(folder: &Path, data_dir: &Path) -> bool {
    let data_dir = std::fs::canonicalize(data_dir).unwrap_or_else(|_| data_dir.to_path_buf());
    folder.starts_with(data_dir)
}

/// The toast at the start when `auto-backup.ini` is there but cannot be
/// read: backups are off until the folder is chosen again.
pub fn unreadable_text() -> String {
    gettext(
        "Automatic backups off: their setting could not be read. Choose the folder again in Preferences.",
    )
}

/// Saves `record` in `data_dir` in one atomic step, mode 0600.
///
/// # Errors
/// The write's error.
pub fn save(data_dir: &Path, record: &Record) -> Result<(), glib::Error> {
    key_file::write_private(&path(data_dir), &to_key_file(record).to_data())
}

/// Turns automatic backups on into `folder`, or off with `None`.
///
/// # Errors
/// The write's error; then the setting is as it was.
pub fn set_folder(data_dir: &Path, folder: Option<&Path>) -> Result<(), glib::Error> {
    let mut record = load(data_dir);
    record.folder = folder.map(Path::to_path_buf);
    save(data_dir, &record)
}

/// Today's date in the user's time zone, as the clock shows it.
pub fn today() -> Option<LocalDate> {
    let now = glib::DateTime::now_local().ok()?;
    let (year, month, day) = now.ymd();
    LocalDate::new(year, month, day)
}

/// The record after a backup to `folder` on `today` ended with `result`.
/// A backup written or found already there is the last backup; a failure
/// is kept for the next start, and the last backup stays as it was, so the
/// next close tries again.
pub fn after(
    mut record: Record,
    folder: &Path,
    today: LocalDate,
    result: Result<&Outcome, (Reason, String)>,
) -> Record {
    record.running = None;
    match result {
        Ok(Outcome::Written {
            fingerprint,
            summary,
            ..
        }) => {
            // A media file missing from this backup is in the fingerprint
            // (the media present), so it gets into a backup once it is back,
            // and an unchanged collection is not backed up again meanwhile.
            if summary.missing_media > 0 {
                tracing::debug!(summary.missing_media, "automatic backup without some media");
            }
            record.last = Some(LastBackup {
                date: today,
                folder: folder.to_path_buf(),
                fingerprint: Some(fingerprint.clone()),
            });
            record.failure = None;
        }
        Ok(Outcome::AlreadyThere) => {
            // What that file holds is not known: the next day backs up
            // whatever changed.
            record.last = Some(LastBackup {
                date: today,
                folder: folder.to_path_buf(),
                fingerprint: None,
            });
            record.failure = None;
        }
        Ok(Outcome::Unchanged) => {
            // Checked today: not again until tomorrow. Unchanged means a
            // last backup to this folder with this fingerprint exists.
            if let Some(last) = record.last.as_mut().filter(|l| l.folder == folder) {
                // Today, not the later of the two: a date ahead of today
                // (a clock set wrong once) is not carried forward.
                last.date = today;
            }
            record.failure = None;
        }
        Err(failure) => record.failure = Some(failure),
    }
    record
}

/// What to report at the start, if anything: a failure recorded, else a
/// backup that started and never ended.
pub fn to_report(record: &Record) -> Option<(Reason, String)> {
    record
        .failure
        .clone()
        .or_else(|| record.running.map(|_| (Reason::Unfinished, String::new())))
}

/// The row's subtitle in Preferences: the folder's name, or "Off".
pub fn folder_subtitle(folder: Option<&Path>) -> String {
    match folder {
        Some(folder) => folder.file_name().map_or_else(
            || folder.display().to_string(),
            |name| name.to_string_lossy().into_owned(),
        ),
        None => pgettext("automatic backups", "Off"),
    }
}

/// Why the backup failed, in words, as the end of [`failed_text`].
pub fn reason_text(reason: Reason) -> String {
    match reason {
        Reason::Failed(FailureKind::FolderMissing) => {
            gettext("the backup folder was not found; is its disk connected?")
        }
        Reason::Failed(FailureKind::NotAFolder) => {
            gettext("the backup folder is not a folder any more; choose it again in Preferences")
        }
        Reason::Failed(FailureKind::FolderIsLink) => {
            gettext("the backup folder was replaced by a link; choose it again in Preferences")
        }
        Reason::Failed(FailureKind::NoSpace) => gettext("the disk is full"),
        Reason::Failed(FailureKind::ReadOnly) => gettext("the backup folder is read-only"),
        Reason::Failed(FailureKind::NotAllowed) => {
            gettext("Rappel may not write to the backup folder")
        }
        Reason::Failed(FailureKind::TooLarge) => gettext(
            "the backup would be larger than an import reads; use Export… without media instead",
        ),
        Reason::Failed(FailureKind::NameTaken) => {
            gettext("something that is not a backup has today’s backup’s name in the backup folder")
        }
        Reason::Failed(FailureKind::Other) | Reason::Stopped => {
            gettext("something unexpected happened")
        }
        Reason::TimedOut | Reason::Failed(FailureKind::Cancelled) => {
            gettext("it took too long and was stopped")
        }
        Reason::Unfinished => gettext("Rappel was stopped before it finished"),
    }
}

/// The toast at the start after a failed backup: "Automatic backup
/// failed: the disk is full" (its title, before ":", fits beside Details
/// at 360 px). An unexpected error says the error itself
/// (in English, as every error detail does).
pub fn failed_text(reason: Reason, detail: &str) -> String {
    let why = match reason {
        Reason::Failed(FailureKind::Other) if !detail.is_empty() => detail.to_owned(),
        _ => reason_text(reason),
    };
    fill(
        // Translators: a toast at the start: what is before ":" is its title, the rest is behind Details. {reason} is why.
        &gettext("Automatic backup failed: {reason}"),
        &[("reason", &why)],
    )
}

/// Shows the toast for the last backup over `window` if it failed or never
/// ended, then forgets it. Not while this process's own backup still runs
/// (the window was opened again while it ran).
pub fn report_at_start(window: &impl IsA<gtk::Widget>, data_dir: &Path) {
    if IN_PROGRESS.get() {
        return;
    }
    let mut record = match load_or_why(data_dir) {
        Ok(record) => record,
        Err(why) => {
            tracing::debug!("the automatic backup setting is not readable: {why}");
            show_error(
                window,
                "reading the automatic backup setting",
                &unreadable_text(),
            );
            return;
        }
    };
    let Some((reason, detail)) = to_report(&record) else {
        return;
    };
    tracing::debug!("the last automatic backup failed: {detail}");
    show_error(window, "an automatic backup", &failed_text(reason, &detail));
    record.failure = None;
    record.running = None;
    if let Err(err) = save(data_dir, &record) {
        tracing::warn!("the automatic backup record could not be saved: {err}");
    }
}

/// Makes the automatic backup if it is due, as the window closes: on a
/// worker thread, with `app` held until it ends or [`TIME_CAP`] passes.
/// Returns at once; the window can close.
pub fn start_at_close(app: &gtk::Application, data_dir: &Path) {
    if IN_PROGRESS.get() {
        return;
    }
    let mut record = load(data_dir);
    let Some(folder) = record.folder.clone() else {
        return;
    };
    let Some(today) = today() else {
        tracing::warn!("no automatic backup: the local date is not known");
        return;
    };
    if !auto_backup::due(record.last.as_ref(), &folder, today) {
        return;
    }
    // Marked as running first, so a backup cut off by a shutdown is
    // reported at the next start.
    record.running = Some(today);
    if let Err(err) = save(data_dir, &record) {
        tracing::warn!("the automatic backup record could not be saved: {err}");
    }
    IN_PROGRESS.set(true);
    // The backup's own hold, dropped when it ends or a while after it is
    // cancelled; never `quit`, which would also end an import or a clean-up
    // that holds the application through `ui::worker::run`. With no other
    // hold and no window, `app.run` returns and the process ends.
    let hold: Rc<RefCell<Option<gio::ApplicationHoldGuard>>> =
        Rc::new(RefCell::new(Some(app.hold())));
    let cancel = Arc::new(AtomicBool::new(false));
    let data_dir = data_dir.to_path_buf();

    // The cap: the backup is cancelled (its next write fails and it removes
    // its partial file) and recorded as cut off; the hold goes after a
    // grace period even if the worker is stuck in the disk.
    let cap: Rc<Cell<Option<glib::SourceId>>> = Rc::default();
    let source = glib::timeout_add_local_once(
        TIME_CAP,
        glib::clone!(
            #[strong]
            cap,
            #[strong]
            cancel,
            #[strong]
            hold,
            #[strong]
            data_dir,
            #[strong]
            folder,
            move || {
                // Fired: it must not be removed again.
                cap.take();
                tracing::warn!("the automatic backup took too long; cancelling it");
                cancel.store(true, Ordering::Relaxed);
                record_end(
                    &data_dir,
                    &folder,
                    today,
                    Err((Reason::TimedOut, String::new())),
                );
                glib::timeout_add_local_once(CANCEL_GRACE, move || {
                    hold.borrow_mut().take();
                });
            }
        ),
    );
    cap.set(Some(source));

    let last = record.last;
    glib::spawn_future_local(async move {
        let (dir, target, flag) = (data_dir.clone(), folder.clone(), Arc::clone(&cancel));
        // `gio::spawn_blocking` returns a panic in the closure as its
        // error (gio's `task.rs`).
        let result = gio::spawn_blocking(move || {
            auto_backup::run(&dir, &target, today, last.as_ref(), &flag)
        })
        .await;
        if let Some(source) = cap.take() {
            source.remove();
        }
        let result = match &result {
            Ok(Ok(outcome)) => {
                tracing::debug!(?outcome, "automatic backup");
                Ok(outcome)
            }
            Ok(Err(Failure { kind, detail })) => {
                tracing::warn!(?kind, "the automatic backup failed");
                Err((Reason::Failed(*kind), detail.clone()))
            }
            Err(_) => {
                tracing::warn!("the automatic backup stopped unexpectedly");
                Err((Reason::Stopped, String::new()))
            }
        };
        record_end(&data_dir, &folder, today, result);
        IN_PROGRESS.set(false);
        hold.borrow_mut().take();
    });
}

/// Saves how the backup ended, on the record as it is on disk now (the
/// folder may have been changed meanwhile in a window opened again).
fn record_end(
    data_dir: &Path,
    folder: &Path,
    today: LocalDate,
    result: Result<&Outcome, (Reason, String)>,
) {
    let record = after(load(data_dir), folder, today, result);
    if let Err(err) = save(data_dir, &record) {
        tracing::warn!("the automatic backup record could not be saved: {err}");
    }
}

#[cfg(test)]
mod tests {
    use rappel_core::interchange::ExportSummary;

    use super::*;

    fn date(text: &str) -> LocalDate {
        LocalDate::parse(text).unwrap()
    }

    const PRINT: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

    fn full() -> Record {
        Record {
            folder: Some(PathBuf::from("/home/me/Backups")),
            last: Some(LastBackup {
                date: date("2026-11-08"),
                folder: PathBuf::from("/home/me/Backups"),
                fingerprint: Some(PRINT.to_owned()),
            }),
            failure: Some((Reason::Failed(FailureKind::NoSpace), "No space".to_owned())),
            running: Some(date("2026-11-09")),
        }
    }

    fn parse(text: &str) -> Record {
        from_key_file(&key_file::parse(text.as_bytes()).unwrap())
    }

    #[test]
    fn a_record_survives_a_round_trip() {
        let record = full();
        assert_eq!(from_key_file(&to_key_file(&record)), record);
        assert_eq!(
            from_key_file(&to_key_file(&Record::default())),
            Record::default()
        );
        for reason in [
            Reason::Failed(FailureKind::FolderMissing),
            Reason::Failed(FailureKind::NotAFolder),
            Reason::Failed(FailureKind::FolderIsLink),
            Reason::Failed(FailureKind::NoSpace),
            Reason::Failed(FailureKind::ReadOnly),
            Reason::Failed(FailureKind::NotAllowed),
            Reason::Failed(FailureKind::TooLarge),
            Reason::Failed(FailureKind::NameTaken),
            Reason::Failed(FailureKind::Other),
            Reason::TimedOut,
            Reason::Unfinished,
            Reason::Stopped,
        ] {
            assert_eq!(Reason::from_code(reason.code()), reason);
        }
    }

    #[test]
    fn bad_values_are_left_out() {
        let record = parse(
            "[settings]\nfolder=relative/path\n\
             [last-backup]\ndate=2026-02-30\nfolder=/x\nfingerprint=abc\n\
             [running]\ndate=yesterday\n",
        );
        assert_eq!(record, Record::default());
        let record = parse(
            "[last-backup]\ndate=2026-11-08\nfolder=/x\nfingerprint=ABC\n\
             [failure]\nreason=from-the-future\n",
        );
        assert_eq!(
            record.last.map(|l| l.fingerprint),
            Some(None),
            "a fingerprint that is not one is unknown"
        );
        assert_eq!(
            record.failure,
            Some((Reason::Failed(FailureKind::Other), String::new())),
            "an unknown reason is still a failure"
        );
    }

    #[test]
    fn load_and_save_use_the_data_directory() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(load(dir.path()), Record::default());
        save(dir.path(), &full()).unwrap();
        assert_eq!(load(dir.path()), full());
        set_folder(dir.path(), None).unwrap();
        let record = load(dir.path());
        assert_eq!(record.folder, None);
        assert_eq!(record.last, full().last, "only the setting changed");
        // A link in place of the file is not followed.
        let file = dir.path().join(FILE_NAME);
        let elsewhere = dir.path().join("elsewhere.ini");
        std::fs::rename(&file, &elsewhere).unwrap();
        std::os::unix::fs::symlink(&elsewhere, &file).unwrap();
        assert_eq!(load(dir.path()), Record::default());
        // Garbage: backups off.
        std::fs::remove_file(&file).unwrap();
        std::fs::write(&file, b"\xff\xfe").unwrap();
        assert_eq!(load(dir.path()), Record::default());
    }

    #[test]
    fn a_long_error_is_cut() {
        let mut record = full();
        record.failure = Some((Reason::Failed(FailureKind::Other), "é".repeat(5000)));
        let detail = from_key_file(&to_key_file(&record)).failure.unwrap().1;
        assert_eq!(detail.chars().count(), MAX_DETAIL_CHARS);
    }

    #[test]
    fn the_record_after_a_backup() {
        let folder = Path::new("/home/me/Backups");
        let today = date("2026-11-09");
        let summary = ExportSummary {
            decks: 1,
            cards: 1,
            tags: 0,
            review_events: 0,
            media: 0,
            media_left_out: 0,
            missing_media: 0,
        };
        let written = Outcome::Written {
            file_name: "Rappel backup 2026-11-09.rappel".to_owned(),
            fingerprint: PRINT.to_owned(),
            summary: ExportSummary {
                decks: 1,
                cards: 1,
                tags: 0,
                review_events: 0,
                media: 0,
                media_left_out: 0,
                missing_media: 0,
            },
            removed: 0,
        };
        let record = after(full(), folder, today, Ok(&written));
        assert_eq!(record.running, None);
        assert_eq!(record.failure, None);
        assert_eq!(
            record.last,
            Some(LastBackup {
                date: today,
                folder: folder.to_path_buf(),
                fingerprint: Some(PRINT.to_owned()),
            })
        );
        assert_eq!(record.folder, full().folder, "the setting is kept");

        let record = after(full(), folder, today, Ok(&Outcome::AlreadyThere));
        assert_eq!(
            record.last.map(|l| (l.date, l.fingerprint)),
            Some((today, None))
        );

        let record = after(full(), folder, today, Ok(&Outcome::Unchanged));
        assert_eq!(
            record
                .last
                .as_ref()
                .map(|l| (l.date, l.fingerprint.as_deref())),
            Some((today, Some(PRINT))),
            "checked today: not again until tomorrow"
        );
        assert_eq!(record.failure, None);
        // Unchanged against a backup in another folder (cannot happen:
        // `changed` says yes then) leaves the record alone.
        let elsewhere = after(full(), Path::new("/x"), today, Ok(&Outcome::Unchanged));
        assert_eq!(elsewhere.last, full().last);

        // A backup with media missing is not trusted as complete.
        let missing = Outcome::Written {
            file_name: "Rappel backup 2026-11-09.rappel".to_owned(),
            fingerprint: PRINT.to_owned(),
            summary: ExportSummary {
                missing_media: 1,
                ..summary
            },
            removed: 0,
        };
        let record = after(full(), folder, today, Ok(&missing));
        assert_eq!(
            record.last.map(|l| l.fingerprint),
            Some(Some(PRINT.to_owned())),
            "the fingerprint has the media present: kept"
        );
        // A last backup dated far ahead (a wrong clock) is not carried
        // forward by an unchanged check.
        let mut ahead = full();
        if let Some(last) = ahead.last.as_mut() {
            last.date = date("2030-01-01");
        }
        let record = after(ahead, folder, today, Ok(&Outcome::Unchanged));
        assert_eq!(record.last.map(|l| l.date), Some(today));
        // Cancelled by the cap reads back as "took too long".
        assert_eq!(
            Reason::from_code(Reason::Failed(FailureKind::Cancelled).code()),
            Reason::TimedOut
        );

        let failed = (
            Reason::Failed(FailureKind::FolderMissing),
            "gone".to_owned(),
        );
        let record = after(full(), folder, today, Err(failed.clone()));
        assert_eq!(record.failure, Some(failed));
        assert_eq!(record.last, full().last, "tried again at the next close");
        assert_eq!(record.running, None);
    }

    #[test]
    fn a_folder_inside_the_data_directory_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let data = dir.path().join("data");
        std::fs::create_dir_all(data.join("backups")).unwrap();
        let data = std::fs::canonicalize(&data).unwrap();
        assert!(inside_data_dir(&data, &data));
        assert!(inside_data_dir(&data.join("backups"), &data));
        assert!(inside_data_dir(&data.join("media"), &data));
        assert!(!inside_data_dir(&dir.path().join("data-backups"), &data));
        assert!(!inside_data_dir(dir.path(), &data));
        // Through a link to the data directory: still inside.
        let link = dir.path().join("link");
        std::os::unix::fs::symlink(&data, &link).unwrap();
        assert!(inside_data_dir(&data.join("backups"), &link));
    }

    #[test]
    fn an_unreadable_setting_is_said() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(load_or_why(dir.path()), Ok(Record::default()));
        std::fs::write(dir.path().join(FILE_NAME), b"\xff\xfe").unwrap();
        assert!(load_or_why(dir.path()).is_err());
        assert_eq!(load(dir.path()), Record::default(), "backups off");
        // Its title fits beside Details at 360 px.
        assert!(matches!(
            crate::ui::toast::toast_text(&unreadable_text()),
            crate::ui::toast::ToastText::Detailed { title, .. } if title == "Automatic backups off"
        ));
    }

    #[test]
    fn what_the_start_reports() {
        assert_eq!(to_report(&Record::default()), None);
        assert_eq!(
            to_report(&full()),
            Some((Reason::Failed(FailureKind::NoSpace), "No space".to_owned()))
        );
        let running = Record {
            failure: None,
            ..full()
        };
        assert_eq!(
            to_report(&running),
            Some((Reason::Unfinished, String::new()))
        );
    }

    #[test]
    fn texts() {
        assert_eq!(folder_subtitle(None), "Off");
        assert_eq!(
            folder_subtitle(Some(Path::new("/home/me/My Backups"))),
            "My Backups"
        );
        // A document portal path shows the folder's own name.
        assert_eq!(
            folder_subtitle(Some(Path::new("/run/user/1000/doc/1a2b3c4d/Backups"))),
            "Backups"
        );
        assert_eq!(folder_subtitle(Some(Path::new("/"))), "/");
        assert_eq!(
            failed_text(Reason::Failed(FailureKind::NoSpace), "ENOSPC"),
            "Automatic backup failed: the disk is full"
        );
        assert_eq!(
            failed_text(Reason::Failed(FailureKind::Other), "database is locked"),
            "Automatic backup failed: database is locked"
        );
        assert_eq!(
            failed_text(Reason::Failed(FailureKind::Other), ""),
            "Automatic backup failed: something unexpected happened"
        );
        // A title of its own beside Details, at 360 px (ADR 0058).
        assert!(matches!(
            crate::ui::toast::toast_text(&failed_text(Reason::Failed(FailureKind::NoSpace), "")),
            crate::ui::toast::ToastText::Detailed { title, .. } if title == "Automatic backup failed"
        ));
        assert_eq!(
            failed_text(Reason::Unfinished, ""),
            "Automatic backup failed: Rappel was stopped before it finished"
        );
    }
}
