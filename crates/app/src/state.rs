//! `AppState`: the one owner of the open database. The UI calls its methods
//! and never sees SQL or rusqlite (ADR 0011).

use std::cell::RefCell;
use std::collections::HashMap;
use std::fmt;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::atomic::AtomicBool;
use std::time::Duration;

use rappel_core::domain::{
    ArchivedDeck, Card, CardState, Deck, DeckSummary, Grade, ReviewEvent, SettingKey, Settings,
    Tag, TaggedCard,
};
use rappel_core::interchange::{
    AnkiPlan, AnkiReport, ExportOptions, ExportSummary, FileKind, ImportPlan, ImportReport,
    MediaTotals, file_kind,
};
use rappel_core::media::{CleanedUp, MediaRef, TrashTidied, UnusedMedia};
use rappel_core::scheduling::{StudyInput, StudyScope};
use rappel_core::stats::{CardInfo, DayGrid, Stats};
use rappel_core::storage::{
    self, BackupInfo, BrowseQuery, BrowseRow, CardStanding, Merged, Moved, OpenWarning, Restored,
    SameFront, Scope, Store, TagsAdded,
};

use crate::clock::{local_day_bounds_ms, local_day_start_ms, local_next_day_start_ms};

/// How many days a period of the statistics covers: the past ones, the
/// current one ending today, and the forecast, from tomorrow (ADR 0032,
/// 0078).
pub const STATS_DAYS: usize = 30;
/// The furthest period back the statistics read: about 82 years, so the
/// day offsets stay small; the page stops at the first review anyway.
pub const STATS_MAX_PERIOD: u32 = 1_000;

/// The statistics of a deck or of everything for one period, with the
/// study days they are bucketed by.
#[derive(Debug, Clone, PartialEq)]
pub struct StatsData {
    /// `days` are the period's; `forecast` the `STATS_DAYS` days from
    /// tomorrow (today's due cards and the overdue ones are left out,
    /// `overdue` still counts the latter).
    pub stats: Stats,
    /// Where each of the period's days begins, UTC milliseconds
    /// (`STATS_DAYS` of them, oldest first).
    pub past_starts: Vec<i64>,
    /// Where each of the forecast's days begins, tomorrow first
    /// (`STATS_DAYS` of them).
    pub ahead_starts: Vec<i64>,
    /// When the earliest review the activity counts was made, if any.
    pub first_review_at: Option<i64>,
}

/// What the editor takes from a file or bytes (ADR 0055, 0056).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Accept {
    /// Only an image (Insert Image…, a picture): `Store::add_image`.
    Image,
    /// Only audio (Insert Audio…): `Store::add_audio`.
    Audio,
    /// An image or audio, whichever the first bytes say (files pasted or
    /// dropped): `Store::add_media`.
    Any,
}

/// Why a media file could not be added in the editor.
#[derive(Debug)]
pub enum AddMediaError {
    /// Not a file on this computer (a URL a drop or the clipboard gave):
    /// never read.
    NotLocal,
    /// Not a regular file (a folder, a pipe, a device).
    NotAFile,
    /// A picture from the clipboard or a drop larger than an image may be.
    TooLarge,
    /// The file could not be opened or read.
    Unreadable(std::io::Error),
    /// The store refused it or failed (`Store::add_media` and its kinds).
    Store(rappel_core::Error),
}

/// The path of `file` if it is a file on this computer: a `file:` URI
/// with a path. Decided without any IO.
pub fn local_path(file: &gtk::gio::File) -> Option<PathBuf> {
    use gtk::prelude::*;
    if file.has_uri_scheme("file") {
        file.path()
    } else {
        None
    }
}

/// A cheap-to-clone handle on the open database, shared by the window and
/// its widgets.
///
/// `Rc` because GTK runs every widget on the main thread, so there is no
/// second thread to share with. `RefCell` because writes need `&mut Store`.
/// Every method borrows the store and releases it before returning; none
/// returns a borrow or calls back into GTK while holding one, so a borrow
/// can never fail at run time.
#[derive(Clone)]
pub struct AppState(Rc<Inner>);

struct Inner {
    data_dir: PathBuf,
    store: RefCell<Store>,
}

impl fmt::Debug for AppState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AppState")
            .field("data_dir", &self.0.data_dir)
            .finish_non_exhaustive()
    }
}

impl AppState {
    /// Opens the database in `data_dir` (see [`Store::open`]).
    pub fn open(data_dir: PathBuf) -> rappel_core::Result<Self> {
        let store = Store::open(&data_dir)?;
        Ok(Self::new(data_dir, store))
    }

    /// Wraps a store that is already open, e.g. after a restore.
    pub fn new(data_dir: PathBuf, store: Store) -> Self {
        Self(Rc::new(Inner {
            data_dir,
            store: RefCell::new(store),
        }))
    }

    /// The backups in `data_dir`, newest first. An associated function: it
    /// is used by the recovery page, before any database is open.
    pub fn list_backups(data_dir: &Path) -> rappel_core::Result<Vec<BackupInfo>> {
        storage::list_backups(data_dir)
    }

    /// Restores `backup` in `data_dir` when no database could be opened
    /// (see [`storage::restore_backup`]). Blocking; runs on a worker thread.
    pub fn restore_unopened(data_dir: &Path, backup: &BackupInfo) -> rappel_core::Result<Restored> {
        storage::restore_backup(data_dir, None, backup)
    }

    /// Installs the checked copy an interrupted restore left waiting in
    /// `data_dir` (see [`storage::finish_restore`]). Blocking; runs on a
    /// worker thread.
    pub fn finish_restore_unopened(data_dir: &Path) -> rappel_core::Result<Restored> {
        storage::finish_restore(data_dir, None)
    }

    /// Moves the database in `data_dir` aside and starts an empty one, when
    /// none could be opened (see [`storage::start_fresh`]). Blocking; runs
    /// on a worker thread.
    pub fn start_fresh_unopened(data_dir: &Path) -> rappel_core::Result<Restored> {
        storage::start_fresh(data_dir, None)
    }

    /// Puts the default back for the damaged stored setting `key` in
    /// `data_dir`, after a backup, and opens the database (see
    /// [`storage::reset_setting`], ADR 0052). Blocking; runs on a worker
    /// thread, before any database is open.
    pub fn reset_setting_unopened(
        data_dir: &Path,
        key: SettingKey,
    ) -> rappel_core::Result<storage::SettingReset> {
        storage::reset_setting(data_dir, key)
    }

    /// Exports the database in `data_dir` to `path` (see
    /// [`Store::export_to`]). Blocking; runs on a worker thread, on its own
    /// connection ([`Store::open_concurrent`], ADR 0018), so the window's
    /// store stays on the main thread.
    pub fn export_on_worker(
        data_dir: &Path,
        path: &Path,
        include_media: bool,
    ) -> rappel_core::Result<ExportSummary> {
        Store::open_concurrent(data_dir)?.export_to_with(path, ExportOptions { include_media })
    }

    /// The media files and their sizes together (see
    /// [`Store::media_totals`]): one query, asked before an export.
    pub fn media_totals(&self) -> rappel_core::Result<MediaTotals> {
        self.0.store.borrow().media_totals()
    }

    /// Checks the `.rappel` file `path` against the database in `data_dir`
    /// (see [`Store::plan_import_cancellable`]); stops when `cancel` is
    /// set. Blocking; on its own connection like [`Self::export_on_worker`].
    pub fn plan_import_on_worker(
        data_dir: &Path,
        path: &Path,
        cancel: &AtomicBool,
    ) -> rappel_core::Result<ImportPlan> {
        Store::open_concurrent(data_dir)?.plan_import_cancellable(path, cancel)
    }

    /// Whether `path` is a Rappel export or an Anki package, by its name,
    /// else by its content ([`file_kind`]). Blocking: it may read the
    /// file's list of entries.
    pub fn file_kind_on_worker(path: &Path) -> Option<FileKind> {
        file_kind(path)
    }

    /// Reads the Anki package `path` and compares it with the database in
    /// `data_dir` (see [`Store::plan_anki_import`]); stops when `cancel` is
    /// set. Blocking; on its own connection like [`Self::export_on_worker`].
    pub fn plan_anki_import_on_worker(
        data_dir: &Path,
        path: &Path,
        cancel: &AtomicBool,
    ) -> rappel_core::Result<AnkiPlan> {
        Store::open_concurrent(data_dir)?.plan_anki_import(path, cancel)
    }

    /// Applies an Anki import `plan` (see [`Store::apply_anki_import`]);
    /// writes nothing if `cancel` is set before it commits. Blocking; on
    /// its own connection like [`Self::export_on_worker`].
    pub fn apply_anki_import_on_worker(
        data_dir: &Path,
        plan: &AnkiPlan,
        cancel: &AtomicBool,
    ) -> rappel_core::Result<AnkiReport> {
        Store::open_concurrent(data_dir)?.apply_anki_import(plan, cancel)
    }

    /// Applies `plan` to the database in `data_dir` (see
    /// [`Store::apply_import`]). Blocking; on its own connection like
    /// [`Self::export_on_worker`]. A change made meanwhile on the main
    /// connection refuses it with `ImportPlanStale`, and a change to the
    /// file since it was checked with `ImportFileChanged`.
    pub fn apply_import_on_worker(
        data_dir: &Path,
        plan: &ImportPlan,
    ) -> rappel_core::Result<ImportReport> {
        Store::open_concurrent(data_dir)?.apply_import(plan)
    }

    /// Sets the desired retention in the database in `data_dir` and rebuilds
    /// every card's schedule from the review log (see
    /// [`Store::set_desired_retention`]). Blocking, as long as the log is
    /// big; runs on a worker thread, on its own connection like
    /// [`Self::export_on_worker`].
    pub fn set_desired_retention_on_worker(data_dir: &Path, value: f32) -> rappel_core::Result<()> {
        Store::open_concurrent(data_dir)?.set_desired_retention(value)
    }

    /// Copies the media file `file` into the media store of the database
    /// in `data_dir` (see [`Store::add_media`]), if it is of a kind
    /// `accept` takes, and returns its reference. Blocking (a file of up
    /// to 20 MB is hashed, copied and synced); runs on a worker thread, on
    /// its own connection like [`Self::export_on_worker`].
    ///
    /// Only a file on this computer is read ([`local_path`]): a `GFile`
    /// from a drop or the clipboard can be any URI (`https:`, `sftp:`,
    /// `smb:`), and reading one through `GVfs` would fetch it over the
    /// network (SPEC principle 5). A file of the document portal is a
    /// local path. It is opened without blocking (a pipe would otherwise
    /// hang the open) and must be a regular file, checked on the open
    /// handle, before a byte is read.
    pub fn add_media_on_worker(
        data_dir: &Path,
        file: &gtk::gio::File,
        accept: Accept,
    ) -> Result<MediaRef, AddMediaError> {
        use std::os::unix::fs::OpenOptionsExt;
        let path = local_path(file).ok_or(AddMediaError::NotLocal)?;
        let opened = std::fs::OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NONBLOCK)
            .open(&path)
            .map_err(AddMediaError::Unreadable)?;
        let meta = opened.metadata().map_err(AddMediaError::Unreadable)?;
        if !meta.file_type().is_file() {
            return Err(AddMediaError::NotAFile);
        }
        Self::add_to_store(data_dir, opened, accept)
    }

    /// [`Self::add_media_on_worker`] for bytes (a pasted or dropped
    /// picture, as the other program gave it).
    pub fn add_media_bytes_on_worker(
        data_dir: &Path,
        bytes: &[u8],
        accept: Accept,
    ) -> Result<MediaRef, AddMediaError> {
        Self::add_to_store(data_dir, bytes, accept)
    }

    fn add_to_store(
        data_dir: &Path,
        reader: impl std::io::Read,
        accept: Accept,
    ) -> Result<MediaRef, AddMediaError> {
        Store::open_concurrent(data_dir)
            .and_then(|store| match accept {
                Accept::Image => store.add_image(reader),
                Accept::Audio => store.add_audio(reader),
                Accept::Any => store.add_media(reader),
            })
            .map_err(AddMediaError::Store)
    }

    /// The media files no card uses in `data_dir`, and their size (see
    /// [`Store::unused_media`], ADR 0057). Blocking (every card is read);
    /// on its own connection like [`Self::export_on_worker`].
    pub fn unused_media_on_worker(data_dir: &Path) -> rappel_core::Result<UnusedMedia> {
        Store::open_concurrent(data_dir)?.unused_media()
    }

    /// Moves the media files no card uses in `data_dir` to the trash (see
    /// [`Store::clean_up_media`]). Blocking; on its own connection like
    /// [`Self::export_on_worker`].
    pub fn clean_up_media_on_worker(data_dir: &Path) -> rappel_core::Result<CleanedUp> {
        Store::open_concurrent(data_dir)?.clean_up_media()
    }

    /// Puts back what `cleaned` moved, as far as it can (see
    /// [`Store::undo_clean_up`]). Blocking; on its own connection.
    pub fn undo_clean_up_on_worker(
        data_dir: &Path,
        cleaned: &CleanedUp,
    ) -> rappel_core::Result<usize> {
        Store::open_concurrent(data_dir)?.undo_clean_up(cleaned)
    }

    /// At startup: brings back the trash's files the database uses and
    /// purges the old ones nothing uses (see [`Store::tidy_media_trash`]).
    /// Blocking (it may read every backup); on its own connection.
    pub fn tidy_media_trash_on_worker(data_dir: &Path) -> rappel_core::Result<TrashTidied> {
        // A temporary copy an Anki import left after a crash goes too.
        rappel_core::interchange::remove_stale_anki_temps(data_dir);
        Store::open_concurrent(data_dir)?.tidy_media_trash()
    }

    /// The media folder, where card images and audio are read from (it may not
    /// exist yet). `None` for a store without a data directory.
    pub fn media_folder(&self) -> Option<PathBuf> {
        self.0.store.borrow().media_folder().map(Path::to_path_buf)
    }

    /// The data directory the database lives in.
    pub fn data_dir(&self) -> &Path {
        &self.0.data_dir
    }

    /// The live decks with what each has to study at `now`, counted like
    /// the deck page (see [`Store::list_decks`]).
    pub fn list_decks(&self, now: i64) -> rappel_core::Result<Vec<DeckSummary>> {
        let store = self.0.store.borrow();
        let hour = store.settings()?.day_start_hour;
        store.list_decks(now, local_day_start_ms(now, hour))
    }

    /// The live decks that are not archived, without counting their cards
    /// (see [`Store::decks`]): what cards can be moved or merged into. The
    /// deck list's rows use [`Self::list_decks`].
    pub fn decks(&self) -> rappel_core::Result<Vec<Deck>> {
        self.0.store.borrow().decks()
    }

    /// Every live deck, archived ones included (see
    /// [`Store::decks_with_archived`]): for names (the name check, All
    /// Cards' deck filter, import conflicts).
    pub fn decks_with_archived(&self) -> rappel_core::Result<Vec<Deck>> {
        self.0.store.borrow().decks_with_archived()
    }

    /// The archived decks with their card counts (see
    /// [`Store::archived_decks`]).
    pub fn archived_decks(&self) -> rappel_core::Result<Vec<ArchivedDeck>> {
        self.0.store.borrow().archived_decks()
    }

    /// Archives a deck (see [`Store::archive_deck`]).
    pub fn archive_deck(&self, id: &str) -> rappel_core::Result<Deck> {
        self.0.store.borrow_mut().archive_deck(id)
    }

    /// Unarchives a deck, the Undo of [`Self::archive_deck`] (see
    /// [`Store::unarchive_deck`]).
    pub fn unarchive_deck(&self, id: &str) -> rappel_core::Result<Deck> {
        self.0.store.borrow_mut().unarchive_deck(id)
    }

    /// Live deck `id`, archived or not, without its counts (see
    /// [`Store::deck`]).
    pub fn deck(&self, id: &str) -> rappel_core::Result<Deck> {
        self.0.store.borrow().deck(id)
    }

    /// Creates a deck (see [`Store::create_deck`]).
    pub fn create_deck(&self, name: &str) -> rappel_core::Result<Deck> {
        self.0.store.borrow_mut().create_deck(name)
    }

    /// Renames a deck (see [`Store::rename_deck`]).
    pub fn rename_deck(&self, id: &str, name: &str) -> rappel_core::Result<()> {
        self.0.store.borrow_mut().rename_deck(id, name)
    }

    /// Soft-deletes a deck and its cards (see [`Store::delete_deck`]).
    pub fn delete_deck(&self, id: &str) -> rappel_core::Result<()> {
        self.0.store.borrow_mut().delete_deck(id)
    }

    /// Undoes [`Self::delete_deck`] (see [`Store::undelete_deck`]).
    pub fn undelete_deck(&self, id: &str) -> rappel_core::Result<Deck> {
        self.0.store.borrow_mut().undelete_deck(id)
    }

    /// How many live cards a deck has (see [`Store::card_count`]).
    pub fn card_count(&self, deck_id: &str) -> rappel_core::Result<usize> {
        self.0.store.borrow().card_count(deck_id)
    }

    /// Merges a deck into another (see [`Store::merge_deck`]).
    pub fn merge_deck(&self, source: &str, target: &str) -> rappel_core::Result<Merged> {
        self.0.store.borrow_mut().merge_deck(source, target)
    }

    /// Undoes [`Self::merge_deck`] (see [`Store::unmerge_deck`]).
    pub fn unmerge_deck(&self, merged: &Merged) -> rappel_core::Result<Deck> {
        self.0.store.borrow_mut().unmerge_deck(merged)
    }

    /// The live cards of a deck, oldest first (see [`Store::list_cards`]).
    pub fn list_cards(&self, deck_id: &str) -> rappel_core::Result<Vec<Card>> {
        self.0.store.borrow().list_cards(deck_id)
    }

    /// The live cards of a deck, oldest first, with their tags (see
    /// [`Store::list_tagged_cards`]).
    pub fn list_tagged_cards(&self, deck_id: &str) -> rappel_core::Result<Vec<TaggedCard>> {
        self.0.store.borrow().list_tagged_cards(deck_id)
    }

    /// Adds a card with exactly these tags, in one write (see
    /// [`Store::add_card_with_tags`]).
    pub fn add_card_with_tags(
        &self,
        deck_id: &str,
        front: &str,
        back: &str,
        tag_names: &[String],
    ) -> rappel_core::Result<TaggedCard> {
        self.0
            .store
            .borrow_mut()
            .add_card_with_tags(deck_id, front, back, tag_names)
    }

    /// Replaces a card's front and back and, with `tag_names`, its tags, in
    /// one write (see [`Store::edit_card_with_tags`]).
    pub fn edit_card_with_tags(
        &self,
        id: &str,
        front: &str,
        back: &str,
        tag_names: Option<&[String]>,
    ) -> rappel_core::Result<TaggedCard> {
        self.0
            .store
            .borrow_mut()
            .edit_card_with_tags(id, front, back, tag_names)
    }

    /// The tags a live card has, for the suggestions and the tag filter
    /// (see [`Store::used_tags`], ADR 0052).
    pub fn used_tags(&self) -> rappel_core::Result<Vec<Tag>> {
        self.0.store.borrow().used_tags()
    }

    /// The tags in use, the cards of archived decks counted only with
    /// `include_archived` (see [`Store::used_tags_listed`], ADR 0068).
    pub fn used_tags_listed(&self, include_archived: bool) -> rappel_core::Result<Vec<Tag>> {
        self.0.store.borrow().used_tags_listed(include_archived)
    }

    /// Whether any live card is left, archived decks included (see
    /// [`Store::has_live_cards`]).
    pub fn has_live_cards(&self) -> rappel_core::Result<bool> {
        self.0.store.borrow().has_live_cards()
    }

    /// Every tag, unused ones too (see [`Store::tags`]).
    pub fn tags(&self) -> rappel_core::Result<Vec<Tag>> {
        self.0.store.borrow().tags()
    }

    /// The tags of a live card (see [`Store::card_tags`]).
    pub fn card_tags(&self, card_id: &str) -> rappel_core::Result<Vec<Tag>> {
        self.0.store.borrow().card_tags(card_id)
    }

    /// Card `id` (see [`Store::card`]).
    pub fn card(&self, id: &str) -> rappel_core::Result<Card> {
        self.0.store.borrow().card(id)
    }

    /// What `scope` (a deck or a tag) has to study at `now`, with the study day starting at
    /// the settings' day start hour, local time (`local_day_start_ms`; see
    /// [`Store::study_input`]).
    pub fn study_input(&self, scope: &StudyScope, now: i64) -> rappel_core::Result<StudyInput> {
        let store = self.0.store.borrow();
        let hour = store.settings()?.day_start_hour;
        store.study_input(scope, now, local_day_start_ms(now, hour))
    }

    /// The cards a practice of `scope` goes over at `now`, in order (see
    /// [`Store::practice_queue`]). A shared borrow of the store: practice
    /// reads and never writes (ADR 0070).
    pub fn practice_queue(&self, scope: &StudyScope, now: i64) -> rappel_core::Result<Vec<String>> {
        self.0.store.borrow().practice_queue(scope, now)
    }

    /// The store's [`Store::total_changes`], for tests that check that a
    /// practice wrote nothing.
    #[cfg(test)]
    pub fn total_changes(&self) -> u64 {
        self.0.store.borrow().total_changes()
    }

    /// Whether a practice of `scope` would show a card (see
    /// [`Store::has_practisable`]); a shared borrow, as `practice_queue`.
    pub fn has_practisable(&self, scope: &StudyScope) -> rappel_core::Result<bool> {
        self.0.store.borrow().has_practisable(scope)
    }

    /// When the next study day begins after `now`, UTC milliseconds (the
    /// day start hour of the settings, local time).
    pub fn next_day_start(&self, now: i64) -> rappel_core::Result<i64> {
        let hour = self.0.store.borrow().settings()?.day_start_hour;
        Ok(local_next_day_start_ms(now, hour))
    }

    /// The settings (see [`Store::settings`]).
    pub fn settings(&self) -> rappel_core::Result<Settings> {
        self.0.store.borrow().settings()
    }

    /// Sets the new cards per deck and day (see
    /// [`Store::set_max_new_per_day`]).
    pub fn set_max_new_per_day(&self, value: u32) -> rappel_core::Result<()> {
        self.0.store.borrow_mut().set_max_new_per_day(value)
    }

    /// Sets the reviews per deck and day (see
    /// [`Store::set_max_reviews_per_day`]).
    pub fn set_max_reviews_per_day(&self, value: u32) -> rappel_core::Result<()> {
        self.0.store.borrow_mut().set_max_reviews_per_day(value)
    }

    /// Sets the hour a study day begins (see [`Store::set_day_start_hour`]).
    pub fn set_day_start_hour(&self, value: u8) -> rappel_core::Result<()> {
        self.0.store.borrow_mut().set_day_start_hour(value)
    }

    /// The folder backups are kept in.
    pub fn backup_dir(&self) -> PathBuf {
        storage::backup_dir(&self.0.data_dir)
    }

    /// When what a deck has to study next changes (see
    /// [`Store::next_study_change`]).
    pub fn next_study_change(
        &self,
        scope: &StudyScope,
        now: i64,
    ) -> rappel_core::Result<Option<i64>> {
        self.0.store.borrow().next_study_change(scope, now)
    }

    /// Records an answer (see [`Store::record_review`]).
    pub fn record_review(
        &self,
        card_id: &str,
        grade: Grade,
        duration: Duration,
        now: i64,
    ) -> rappel_core::Result<ReviewEvent> {
        self.0
            .store
            .borrow_mut()
            .record_review(card_id, grade, duration, now)
    }

    /// Makes a card new again, keeping its history (see
    /// [`Store::reset_card`]).
    pub fn reset_card(&self, card_id: &str, now: i64) -> rappel_core::Result<ReviewEvent> {
        self.0.store.borrow_mut().reset_card(card_id, now)
    }

    /// Cancels the card's latest review or reset (see [`Store::undo`]).
    pub fn undo(
        &self,
        card_id: &str,
        event_id: &str,
        now: i64,
    ) -> rappel_core::Result<ReviewEvent> {
        self.0.store.borrow_mut().undo(card_id, event_id, now)
    }

    /// A card's whole log (see [`Store::review_events`]); tests check it.
    #[cfg(test)]
    pub fn review_events(&self, card_id: &str) -> rappel_core::Result<Vec<ReviewEvent>> {
        self.0.store.borrow().review_events(card_id)
    }

    /// The statistics of `deck_id`, or of every deck for `None`, at `now`,
    /// for `period`: 0 is the `STATS_DAYS` days ending with today, 1 the
    /// ones before them, and so on (clamped to `STATS_MAX_PERIOD`). The
    /// forecast is the `STATS_DAYS` days from tomorrow whatever the period.
    /// The days follow the settings' day start hour in local time (see
    /// [`Store::stats_and_first_activity`]).
    ///
    /// # Errors
    /// A missing or deleted deck, or a failed read; a day grid that cannot
    /// be built (it always can, but core checks it).
    pub fn stats(
        &self,
        deck_id: Option<&str>,
        now: i64,
        period: u32,
    ) -> rappel_core::Result<StatsData> {
        let store = self.0.store.borrow();
        let hour = store.settings()?.day_start_hour;
        let days = i32::try_from(STATS_DAYS).unwrap_or(30);
        // At most 1,001 periods of 30 days back: far inside an i32.
        let back = i32::try_from(period.min(STATS_MAX_PERIOD)).unwrap_or(0) + 1;
        let first_past = 1 - days * back;
        let mut past_bounds = local_day_bounds_ms(now, hour, first_past, STATS_DAYS + 1);
        // Today and the forecast's days: today takes the overdue cards
        // (core's rule), so dropping it leaves the days from tomorrow
        // with only what falls due on them.
        let mut ahead_bounds = local_day_bounds_ms(now, hour, 0, STATS_DAYS + 2);
        let scope = deck_id.map_or(Scope::All, Scope::Deck);
        let (mut stats, first_review_at) = store.stats_and_first_activity(
            scope,
            now,
            &DayGrid::new(past_bounds.clone())?,
            &DayGrid::new(ahead_bounds.clone())?,
        )?;
        past_bounds.pop();
        ahead_bounds.pop();
        // Today is dropped; `get(1..)` is `None` only for an empty list,
        // which a grid core accepted never gives.
        let ahead_bounds = ahead_bounds
            .get(1..)
            .map(<[i64]>::to_vec)
            .unwrap_or_default();
        stats.forecast = stats
            .forecast
            .get(1..)
            .map(<[u32]>::to_vec)
            .unwrap_or_default();
        Ok(StatsData {
            stats,
            past_starts: past_bounds,
            ahead_starts: ahead_bounds,
            first_review_at,
        })
    }

    /// The other live cards whose front reads the same as `front_md` (see
    /// [`Store::cards_with_front`]).
    pub fn cards_with_front(
        &self,
        front_md: &str,
        except_id: Option<&str>,
    ) -> rappel_core::Result<Vec<SameFront>> {
        self.0.store.borrow().cards_with_front(front_md, except_id)
    }

    /// A card's properties and history at `now` (see [`Store::card_info`]).
    pub fn card_info(&self, card_id: &str, now: i64) -> rappel_core::Result<CardInfo> {
        self.0.store.borrow().card_info(card_id, now)
    }

    /// Where each card of a deck stands at `now`, by id (see
    /// [`Store::deck_standings`]).
    pub fn deck_standings(
        &self,
        deck_id: &str,
        now: i64,
    ) -> rappel_core::Result<HashMap<String, CardStanding>> {
        self.0.store.borrow().deck_standings(deck_id, now)
    }

    /// A card's scheduling state (see [`Store::card_state`]).
    pub fn card_state(&self, card_id: &str) -> rappel_core::Result<CardState> {
        self.0.store.borrow().card_state(card_id)
    }

    /// Soft-deletes a card (see [`Store::delete_card`]).
    pub fn delete_card(&self, id: &str) -> rappel_core::Result<()> {
        self.0.store.borrow_mut().delete_card(id)
    }

    /// Undoes [`Self::delete_card`] (see [`Store::undelete_card`]).
    pub fn undelete_card(&self, id: &str) -> rappel_core::Result<Card> {
        self.0.store.borrow_mut().undelete_card(id)
    }

    /// The cards the All Cards page lists for `query`, at `now` (see
    /// [`Store::browse`]).
    pub fn browse(&self, query: &BrowseQuery, now: i64) -> rappel_core::Result<Vec<BrowseRow>> {
        self.0.store.borrow().browse(query, now)
    }

    /// One row of the All Cards page, `None` if the card is gone (see
    /// [`Store::browse_row`]).
    pub fn browse_row(&self, card_id: &str, now: i64) -> rappel_core::Result<Option<BrowseRow>> {
        self.0.store.borrow().browse_row(card_id, now)
    }

    /// Suspends or unsuspends cards; returns those that changed (see
    /// [`Store::set_suspended`]).
    pub fn set_suspended(
        &self,
        ids: &[String],
        suspended: bool,
    ) -> rappel_core::Result<Vec<String>> {
        self.0.store.borrow_mut().set_suspended(ids, suspended)
    }

    /// Undoes [`Self::set_suspended`] (see [`Store::restore_suspended`]).
    pub fn restore_suspended(&self, ids: &[String], suspended: bool) -> rappel_core::Result<usize> {
        self.0.store.borrow_mut().restore_suspended(ids, suspended)
    }

    /// Moves cards to a deck (see [`Store::move_cards`]).
    pub fn move_cards(&self, ids: &[String], deck_id: &str) -> rappel_core::Result<Vec<Moved>> {
        self.0.store.borrow_mut().move_cards(ids, deck_id)
    }

    /// Undoes [`Self::move_cards`] (see [`Store::unmove_cards`]).
    pub fn unmove_cards(&self, moved: &[Moved]) -> rappel_core::Result<usize> {
        self.0.store.borrow_mut().unmove_cards(moved)
    }

    /// Puts tags, created if need be, on cards, leaving out those that would
    /// have more than `max_per_card` (see [`Store::add_tags_to_cards`]).
    pub fn add_tags_to_cards(
        &self,
        ids: &[String],
        names: &[String],
        max_per_card: usize,
    ) -> rappel_core::Result<TagsAdded> {
        self.0
            .store
            .borrow_mut()
            .add_tags_to_cards(ids, names, max_per_card)
    }

    /// Undoes [`Self::add_tags_to_cards`] (see [`Store::release_tags`]).
    pub fn release_tags(&self, links: &[(String, String)]) -> rappel_core::Result<usize> {
        self.0.store.borrow_mut().release_tags(links)
    }

    /// Takes a tag off cards (see [`Store::remove_tag_from_cards`]).
    pub fn remove_tag_from_cards(
        &self,
        ids: &[String],
        tag_id: &str,
    ) -> rappel_core::Result<Vec<String>> {
        self.0.store.borrow_mut().remove_tag_from_cards(ids, tag_id)
    }

    /// Undoes [`Self::remove_tag_from_cards`] (see
    /// [`Store::restore_tag_on_cards`]).
    pub fn restore_tag_on_cards(
        &self,
        card_ids: &[String],
        tag_id: &str,
    ) -> rappel_core::Result<usize> {
        self.0
            .store
            .borrow_mut()
            .restore_tag_on_cards(card_ids, tag_id)
    }

    /// Makes cards new again (see [`Store::reset_cards`]).
    pub fn reset_cards(&self, ids: &[String], now: i64) -> rappel_core::Result<Vec<ReviewEvent>> {
        self.0.store.borrow_mut().reset_cards(ids, now)
    }

    /// Undoes [`Self::reset_cards`]; returns the cards left alone (see
    /// [`Store::undo_entries`]).
    pub fn undo_entries(
        &self,
        entries: &[ReviewEvent],
        now: i64,
    ) -> rappel_core::Result<Vec<String>> {
        self.0.store.borrow_mut().undo_entries(entries, now)
    }

    /// Soft-deletes cards (see [`Store::delete_cards`]).
    pub fn delete_cards(&self, ids: &[String]) -> rappel_core::Result<()> {
        self.0.store.borrow_mut().delete_cards(ids)
    }

    /// Undoes [`Self::delete_cards`] (see [`Store::undelete_cards`]).
    pub fn undelete_cards(&self, ids: &[String]) -> rappel_core::Result<usize> {
        self.0.store.borrow_mut().undelete_cards(ids)
    }

    /// The problems met while opening that did not stop it. Empty after
    /// the first call.
    pub fn take_open_warnings(&self) -> Vec<OpenWarning> {
        self.0.store.borrow_mut().take_open_warnings()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_statistics_cover_thirty_days_ending_today_and_thirty_from_tomorrow() {
        let (_tmp, state) = crate::testing::temporary_state();
        let now = rappel_core::time::now_ms();
        let data = state.stats(None, now, 0).unwrap();
        assert_eq!(data.past_starts.len(), STATS_DAYS);
        assert_eq!(data.ahead_starts.len(), STATS_DAYS);
        assert_eq!(data.stats.days.len(), STATS_DAYS);
        assert_eq!(data.stats.forecast.len(), STATS_DAYS);
        assert_eq!(data.first_review_at, None);
        // Today is the last past day; the forecast begins the day after.
        let today = data.past_starts[STATS_DAYS - 1];
        assert!(today <= now && now < data.ahead_starts[0]);
        assert!(data.past_starts.windows(2).all(|w| w[0] < w[1]));
        assert!(data.ahead_starts.windows(2).all(|w| w[0] < w[1]));
        // The period before ends exactly where this one begins: its days
        // and the end of its last day are the 31 bounds before today's
        // period.
        let before = state.stats(None, now, 1).unwrap();
        assert_eq!(before.past_starts.len(), STATS_DAYS);
        let hour = state.settings().unwrap().day_start_hour;
        let first = 1 - 2 * i32::try_from(STATS_DAYS).unwrap();
        let bounds = local_day_bounds_ms(now, hour, first, STATS_DAYS + 1);
        assert_eq!(bounds[..STATS_DAYS], before.past_starts[..]);
        assert_eq!(bounds[STATS_DAYS], data.past_starts[0]);
        assert_eq!(before.ahead_starts, data.ahead_starts);
        // A period beyond the furthest is read as the furthest.
        assert_eq!(
            state.stats(None, now, u32::MAX).unwrap().past_starts,
            state
                .stats(None, now, STATS_MAX_PERIOD)
                .unwrap()
                .past_starts
        );
    }

    #[test]
    fn the_forecast_leaves_today_and_overdue_cards_out() {
        let (_tmp, state) = crate::testing::temporary_state();
        let deck = state.create_deck("Deck").unwrap();
        let id = state
            .add_card_with_tags(&deck.id, "front", "back", &[])
            .unwrap()
            .card
            .id;
        let now = rappel_core::time::now_ms();
        let long_ago = now - 45 * crate::ui::time::DAY_MS;
        state
            .record_review(
                &id,
                rappel_core::domain::Grade::Fail,
                Duration::from_secs(3),
                long_ago,
            )
            .unwrap();
        let data = state.stats(Some(&deck.id), now, 0).unwrap();
        // Overdue now: counted as such, not on any day from tomorrow.
        assert_eq!(data.stats.overdue, 1);
        assert!(data.stats.forecast.iter().all(|&n| n == 0));
        assert_eq!(data.first_review_at, Some(long_ago));
        assert_eq!(data.stats.days.iter().map(|d| d.reviews).sum::<u32>(), 0);
        let then = state.stats(Some(&deck.id), now, 1).unwrap();
        assert_eq!(then.stats.days.iter().map(|d| d.reviews).sum::<u32>(), 1);
    }

    // Hardening round 5 (ADR 0084): paging the statistics back.

    use crate::pages::stats_text::{can_page_back, nothing_to_show, paged};

    /// Writes `times` (UTC milliseconds, in order) as passed reviews of
    /// card `card_id`, straight into the log as an import would, since one
    /// write per review through the store waits for the disk each time.
    fn log_reviews(state: &AppState, card_id: &str, times: &[i64]) {
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let mut conn = rusqlite::Connection::open(state.data_dir().join("rappel.sqlite3")).unwrap();
        // One transaction: one wait for the disk, not one per review.
        let tx = conn.transaction().unwrap();
        let mut insert = tx
            .prepare(
                "INSERT INTO review_events (id, card_id, kind, reviewed_at, grade, duration_ms)
                 VALUES (?1, ?2, 'review', ?3, 'pass', 1000)",
            )
            .unwrap();
        for at in times {
            let k = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            let id = format!("0ffd0000-0000-7000-8000-{k:012x}");
            insert.execute(rusqlite::params![id, card_id, at]).unwrap();
        }
        drop(insert);
        tx.commit().unwrap();
    }

    /// How many presses of Previous the page allows from the current
    /// period, as the page decides them, up to `limit`.
    fn presses_back(state: &AppState, deck: Option<&str>, now: i64, limit: u32) -> u32 {
        let mut period = 0;
        while period < limit {
            let data = state.stats(deck, now, period).unwrap();
            if !can_page_back(&data, period) {
                break;
            }
            period = paged(period, true);
        }
        period
    }

    /// With no review at all, or only reviews of cards and decks that are
    /// gone, there is nothing to page to; an archived deck's reviews still
    /// count for every deck together (ADR 0068).
    #[test]
    fn paging_back_with_no_reviews_or_only_gone_ones() {
        let (_tmp, state) = crate::testing::temporary_state();
        let now = rappel_core::time::now_ms();
        let day = crate::ui::time::DAY_MS;
        let empty = state.create_deck("Empty").unwrap();
        let data = state.stats(Some(&empty.id), now, 0).unwrap();
        assert!(nothing_to_show(&data));
        assert!(!can_page_back(&data, 0));
        assert_eq!(presses_back(&state, None, now, 5), 0);

        // A deck whose only card was reviewed 90 days ago, then deleted.
        let gone = state.create_deck("Gone").unwrap();
        let card = state
            .add_card_with_tags(&gone.id, "f", "b", &[])
            .unwrap()
            .card;
        log_reviews(&state, &card.id, &[now - 90 * day]);
        state.delete_card(&card.id).unwrap();
        let data = state.stats(Some(&gone.id), now, 0).unwrap();
        assert_eq!(
            data.first_review_at, None,
            "a deleted card's reviews are gone"
        );
        assert!(!can_page_back(&data, 0));
        // A deck archived after a review 70 days ago: its own page and the
        // page of every deck can page back to that review.
        let kept = state.create_deck("Kept").unwrap();
        let card = state
            .add_card_with_tags(&kept.id, "f", "b", &[])
            .unwrap()
            .card;
        log_reviews(&state, &card.id, &[now - 70 * day]);
        state.archive_deck(&kept.id).unwrap();
        assert_eq!(presses_back(&state, Some(&kept.id), now, 10), 2);
        assert_eq!(presses_back(&state, None, now, 10), 2);
        let data = state.stats(None, now, 2).unwrap();
        assert_eq!(data.stats.days.iter().map(|d| d.reviews).sum::<u32>(), 1);
        // The deck deleted too: gone from every page.
        state.delete_deck(&gone.id).unwrap();
        assert_eq!(presses_back(&state, Some(&kept.id), now, 10), 2);
    }

    /// A first review far in the past: paging stops at the furthest period
    /// (`STATS_MAX_PERIOD`, about 82 years), which still reads; one in the
    /// future (made with a clock set ahead, then put right) gives nothing
    /// to page to and is in no day, until the clock passes it.
    #[test]
    fn paging_back_to_a_first_review_far_in_the_past_or_in_the_future() {
        let (_tmp, state) = crate::testing::temporary_state();
        let now = rappel_core::time::now_ms();
        let day = crate::ui::time::DAY_MS;
        let deck = state.create_deck("Deck").unwrap();
        let card = state
            .add_card_with_tags(&deck.id, "f", "b", &[])
            .unwrap()
            .card;
        let year = 365 * day;
        log_reviews(&state, &card.id, &[now - 200 * year, now - 45 * day]);
        let (cpu, presses) =
            crate::testing::cpu_time_of(|| presses_back(&state, Some(&deck.id), now, u32::MAX));
        assert_eq!(presses, STATS_MAX_PERIOD);
        let furthest = state.stats(Some(&deck.id), now, STATS_MAX_PERIOD).unwrap();
        assert_eq!(furthest.past_starts.len(), STATS_DAYS);
        assert!(!can_page_back(&furthest, STATS_MAX_PERIOD));
        crate::testing::print_timing("1,000 statistics periods paged back", cpu);
        crate::testing::assert_cpu_within(cpu, Duration::from_secs(20), "1,000 periods");

        let (_tmp, state) = crate::testing::temporary_state();
        let deck = state.create_deck("Deck").unwrap();
        let card = state
            .add_card_with_tags(&deck.id, "f", "b", &[])
            .unwrap()
            .card;
        log_reviews(&state, &card.id, &[now + 10 * year]);
        let data = state.stats(Some(&deck.id), now, 0).unwrap();
        assert_eq!(data.first_review_at, Some(now + 10 * year));
        assert!(!can_page_back(&data, 0));
        assert!(!nothing_to_show(&data), "the page shows, with empty days");
        assert_eq!(data.stats.days.iter().map(|d| d.reviews).sum::<u32>(), 0);
        // Ten years and two months later it is in the past.
        assert_eq!(
            presses_back(&state, Some(&deck.id), now + 10 * year + 60 * day, 10),
            2
        );
    }

    /// Clocks no real computer shows (before 1970, past `GLib`'s year 9999,
    /// the ends of `i64`) never panic; within `GLib`'s years the statistics
    /// read as usual, beyond them they read or fail with an error, which
    /// the page shows as a toast.
    #[test]
    fn statistics_at_odd_clocks_never_panic() {
        let (_tmp, state) = crate::testing::temporary_state();
        let deck = state.create_deck("Deck").unwrap();
        let card = state
            .add_card_with_tags(&deck.id, "f", "b", &[])
            .unwrap()
            .card;
        log_reviews(&state, &card.id, &[0]);
        let year = 365 * crate::ui::time::DAY_MS;
        for now in [0, -1, -60 * year, 8_000 * year, i64::MIN, i64::MAX] {
            // A panic here fails the test: none may happen.
            for period in [0, 1, STATS_MAX_PERIOD] {
                let read = state.stats(None, now, period).map(|d| d.past_starts.len());
                if (-60 * year..=8_000 * year).contains(&now) && period < STATS_MAX_PERIOD {
                    assert_eq!(read.ok(), Some(STATS_DAYS), "at {now}, period {period}");
                }
            }
            let mut period = 0;
            while let Ok(data) = state.stats(None, now, period)
                && can_page_back(&data, period)
                && period < 3
            {
                period = paged(period, true);
            }
        }
    }

    /// Every review is in exactly one period: consecutive periods meet at
    /// the same instant and none is lost or counted twice, across two
    /// years of daylight-saving changes. Run in a process of its own with
    /// a Central European time zone (a POSIX rule: no tzdata needed).
    #[test]
    fn periods_meet_across_daylight_saving_changes() {
        crate::testing::run_alone("state::tests::periods_meet_in_paris", |command| {
            command
                .env("TZ", "CET-1CEST,M3.5.0,M10.5.0/3")
                .env("RAPPEL_PERIODS_IN_PARIS", "1");
        });
    }

    /// The body of `periods_meet_across_daylight_saving_changes`: it runs
    /// only as that test's child process (`RAPPEL_PERIODS_IN_PARIS`), and
    /// passes at once in a normal run.
    #[test]
    fn periods_meet_in_paris() {
        if std::env::var_os("RAPPEL_PERIODS_IN_PARIS").is_none() {
            return;
        }
        let (_tmp, state) = crate::testing::temporary_state();
        let deck = state.create_deck("Deck").unwrap();
        let card = state
            .add_card_with_tags(&deck.id, "f", "b", &[])
            .unwrap()
            .card;
        // 2026-11-10 12:00 UTC; reviews at each study day's start, a
        // millisecond before it, an hour after it, for 800 days back.
        let now = 1_794_312_000_000;
        let hour = state.settings().unwrap().day_start_hour;
        let starts = local_day_bounds_ms(now, hour, -800, 801);
        let mut times = Vec::new();
        for pair in starts.windows(2) {
            times.extend([pair[0], pair[0] + crate::ui::time::HOUR_MS, pair[1] - 1]);
        }
        log_reviews(&state, &card.id, &times);
        let mut counted = 0;
        let mut newer_start = None;
        let mut lengths = std::collections::BTreeSet::new();
        for period in 0..30 {
            let data = state.stats(Some(&deck.id), now, period).unwrap();
            // Day lengths across the changes: 23, 24 or 25 hours.
            for pair in data.past_starts.windows(2) {
                let length = pair[1] - pair[0];
                assert!(
                    [23, 24, 25]
                        .map(|h| h * crate::ui::time::HOUR_MS)
                        .contains(&length),
                    "a day of {length} ms"
                );
                lengths.insert(length / crate::ui::time::HOUR_MS);
            }
            if let Some(start) = newer_start {
                let end = data.past_starts[STATS_DAYS - 1] + {
                    // This period's last day ends where the newer one starts.
                    let next = local_day_bounds_ms(data.past_starts[STATS_DAYS - 1], hour, 1, 1)[0];
                    next - data.past_starts[STATS_DAYS - 1]
                };
                assert_eq!(end, start, "period {period} meets the one after it");
            }
            newer_start = Some(data.past_starts[0]);
            counted += data.stats.days.iter().map(|d| d.reviews).sum::<u32>();
        }
        assert_eq!(
            lengths.into_iter().collect::<Vec<_>>(),
            [23, 24, 25],
            "the zone's changes were crossed"
        );
        // The current period holds today, which has no review; 800 study
        // days of three reviews each are in the 27 periods before.
        assert_eq!(counted as usize, times.len());
    }
}
