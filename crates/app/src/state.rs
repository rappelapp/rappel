//! `AppState`: the one owner of the open database. The UI calls its methods
//! and never sees SQL or rusqlite (ADR 0011).

use std::cell::RefCell;
use std::fmt;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::Duration;

use rappel_core::domain::{Card, CardState, Deck, DeckSummary, Grade, ReviewEvent, Settings};
use rappel_core::interchange::{ExportSummary, ImportPlan, ImportReport};
use rappel_core::scheduling::StudyInput;
use rappel_core::storage::{self, BackupInfo, OpenWarning, Restored, Store};

use crate::clock::{local_day_start_ms, local_next_day_start_ms};

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

    /// Exports the database in `data_dir` to `path` (see
    /// [`Store::export_to`]). Blocking; runs on a worker thread, on its own
    /// connection ([`Store::open_concurrent`], ADR 0018), so the window's
    /// store stays on the main thread.
    pub fn export_on_worker(data_dir: &Path, path: &Path) -> rappel_core::Result<ExportSummary> {
        Store::open_concurrent(data_dir)?.export_to(path)
    }

    /// Checks the `.rappel` file `path` against the database in `data_dir`
    /// (see [`Store::plan_import`]). Blocking; on its own connection like
    /// [`Self::export_on_worker`].
    pub fn plan_import_on_worker(data_dir: &Path, path: &Path) -> rappel_core::Result<ImportPlan> {
        Store::open_concurrent(data_dir)?.plan_import(path)
    }

    /// Applies `plan` to the database in `data_dir` (see
    /// [`Store::apply_import`]). Blocking; on its own connection like
    /// [`Self::export_on_worker`]. A change made meanwhile on the main
    /// connection refuses it with `ImportPlanStale`.
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

    /// The data directory the database lives in.
    pub fn data_dir(&self) -> &Path {
        &self.0.data_dir
    }

    /// The live decks with what each has to study now, counted like the
    /// deck page (see [`Store::list_decks`]).
    pub fn list_decks(&self) -> rappel_core::Result<Vec<DeckSummary>> {
        let now = rappel_core::time::now_ms();
        let store = self.0.store.borrow();
        let hour = store.settings()?.day_start_hour;
        store.list_decks(now, local_day_start_ms(now, hour))
    }

    /// The live decks, without counting their cards (see [`Store::decks`]).
    /// For names and ids; the deck list's rows use [`Self::list_decks`].
    pub fn decks(&self) -> rappel_core::Result<Vec<Deck>> {
        self.0.store.borrow().decks()
    }

    /// Live deck `id`, without its counts (see [`Store::deck`]).
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

    /// The live cards of a deck, oldest first (see [`Store::list_cards`]).
    pub fn list_cards(&self, deck_id: &str) -> rappel_core::Result<Vec<Card>> {
        self.0.store.borrow().list_cards(deck_id)
    }

    /// Adds a card (see [`Store::add_card`]).
    pub fn add_card(&self, deck_id: &str, front: &str, back: &str) -> rappel_core::Result<Card> {
        self.0.store.borrow_mut().add_card(deck_id, front, back)
    }

    /// Replaces a card's front and back and returns the stored card (see
    /// [`Store::edit_card`]).
    pub fn edit_card(&self, id: &str, front: &str, back: &str) -> rappel_core::Result<Card> {
        self.0.store.borrow_mut().edit_card(id, front, back)
    }

    /// Card `id` (see [`Store::card`]).
    pub fn card(&self, id: &str) -> rappel_core::Result<Card> {
        self.0.store.borrow().card(id)
    }

    /// What a deck has to study at `now`, with the study day starting at
    /// the settings' day start hour, local time (`local_day_start_ms`; see
    /// [`Store::study_input`]).
    pub fn study_input(&self, deck_id: &str, now: i64) -> rappel_core::Result<StudyInput> {
        let store = self.0.store.borrow();
        let hour = store.settings()?.day_start_hour;
        store.study_input(deck_id, now, local_day_start_ms(now, hour))
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
    pub fn next_study_change(&self, deck_id: &str, now: i64) -> rappel_core::Result<Option<i64>> {
        self.0.store.borrow().next_study_change(deck_id, now)
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

    /// The problems met while opening that did not stop it. Empty after
    /// the first call.
    pub fn take_open_warnings(&self) -> Vec<OpenWarning> {
        self.0.store.borrow_mut().take_open_warnings()
    }
}
