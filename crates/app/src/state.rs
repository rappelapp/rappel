//! `AppState`: the one owner of the open database. The UI calls its methods
//! and never sees SQL or rusqlite (ADR 0011).

use std::cell::RefCell;
use std::fmt;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::Duration;

use cards_core::domain::{Card, CardState, Deck, DeckSummary, Grade, ReviewEvent};
use cards_core::scheduling::session::StudyInput;
use cards_core::storage::{self, BackupInfo, OpenWarning, Restored, Store};

use crate::clock::local_day_start_ms;

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
    pub fn open(data_dir: PathBuf) -> cards_core::Result<Self> {
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
    pub fn list_backups(data_dir: &Path) -> cards_core::Result<Vec<BackupInfo>> {
        storage::list_backups(data_dir)
    }

    /// Restores `backup` in `data_dir` when no database could be opened
    /// (see [`storage::restore_backup`]). Blocking; runs on a worker thread.
    pub fn restore_unopened(data_dir: &Path, backup: &BackupInfo) -> cards_core::Result<Restored> {
        storage::restore_backup(data_dir, None, backup)
    }

    /// Moves the database in `data_dir` aside and starts an empty one, when
    /// none could be opened (see [`storage::start_fresh`]). Blocking; runs
    /// on a worker thread.
    pub fn start_fresh_unopened(data_dir: &Path) -> cards_core::Result<Restored> {
        storage::start_fresh(data_dir, None)
    }

    /// The data directory the database lives in.
    pub fn data_dir(&self) -> &Path {
        &self.0.data_dir
    }

    /// The live decks with what each has to study now, counted like the
    /// deck page (see [`Store::list_decks`]).
    pub fn list_decks(&self) -> cards_core::Result<Vec<DeckSummary>> {
        let now = cards_core::time::now_ms();
        self.0
            .store
            .borrow()
            .list_decks(now, local_day_start_ms(now))
    }

    /// Creates a deck (see [`Store::create_deck`]).
    pub fn create_deck(&self, name: &str) -> cards_core::Result<Deck> {
        self.0.store.borrow_mut().create_deck(name)
    }

    /// Renames a deck (see [`Store::rename_deck`]).
    pub fn rename_deck(&self, id: &str, name: &str) -> cards_core::Result<()> {
        self.0.store.borrow_mut().rename_deck(id, name)
    }

    /// Soft-deletes a deck and its cards (see [`Store::delete_deck`]).
    pub fn delete_deck(&self, id: &str) -> cards_core::Result<()> {
        self.0.store.borrow_mut().delete_deck(id)
    }

    /// Undoes [`Self::delete_deck`] (see [`Store::undelete_deck`]).
    pub fn undelete_deck(&self, id: &str) -> cards_core::Result<Deck> {
        self.0.store.borrow_mut().undelete_deck(id)
    }

    /// The live cards of a deck, oldest first (see [`Store::list_cards`]).
    pub fn list_cards(&self, deck_id: &str) -> cards_core::Result<Vec<Card>> {
        self.0.store.borrow().list_cards(deck_id)
    }

    /// Adds a card (see [`Store::add_card`]).
    pub fn add_card(&self, deck_id: &str, front: &str, back: &str) -> cards_core::Result<Card> {
        self.0.store.borrow_mut().add_card(deck_id, front, back)
    }

    /// Replaces a card's front and back and returns the stored card (see
    /// [`Store::edit_card`]).
    pub fn edit_card(&self, id: &str, front: &str, back: &str) -> cards_core::Result<Card> {
        self.0.store.borrow_mut().edit_card(id, front, back)
    }

    /// Card `id` (see [`Store::card`]).
    pub fn card(&self, id: &str) -> cards_core::Result<Card> {
        self.0.store.borrow().card(id)
    }

    /// What a deck has to study (see [`Store::study_input`]).
    pub fn study_input(
        &self,
        deck_id: &str,
        now: i64,
        day_start: i64,
    ) -> cards_core::Result<StudyInput> {
        self.0.store.borrow().study_input(deck_id, now, day_start)
    }

    /// When what a deck has to study next changes (see
    /// [`Store::next_study_change`]).
    pub fn next_study_change(&self, deck_id: &str, now: i64) -> cards_core::Result<Option<i64>> {
        self.0.store.borrow().next_study_change(deck_id, now)
    }

    /// Records an answer (see [`Store::record_review`]).
    pub fn record_review(
        &self,
        card_id: &str,
        grade: Grade,
        duration: Duration,
        now: i64,
    ) -> cards_core::Result<ReviewEvent> {
        self.0
            .store
            .borrow_mut()
            .record_review(card_id, grade, duration, now)
    }

    /// A card's scheduling state (see [`Store::card_state`]).
    pub fn card_state(&self, card_id: &str) -> cards_core::Result<CardState> {
        self.0.store.borrow().card_state(card_id)
    }

    /// Soft-deletes a card (see [`Store::delete_card`]).
    pub fn delete_card(&self, id: &str) -> cards_core::Result<()> {
        self.0.store.borrow_mut().delete_card(id)
    }

    /// Undoes [`Self::delete_card`] (see [`Store::undelete_card`]).
    pub fn undelete_card(&self, id: &str) -> cards_core::Result<Card> {
        self.0.store.borrow_mut().undelete_card(id)
    }

    /// The problems met while opening that did not stop it. Empty after
    /// the first call.
    pub fn take_open_warnings(&self) -> Vec<OpenWarning> {
        self.0.store.borrow_mut().take_open_warnings()
    }
}
