//! `AppState`: the one owner of the open database. The UI calls its methods
//! and never sees SQL or rusqlite (ADR 0011).

use std::cell::RefCell;
use std::fmt;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use cards_core::domain::DeckSummary;
use cards_core::storage::{self, BackupInfo, OpenWarning, Restored, Store};

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

    /// The data directory the database lives in.
    pub fn data_dir(&self) -> &Path {
        &self.0.data_dir
    }

    /// The live decks with their new and due counts as of now.
    pub fn list_decks(&self) -> cards_core::Result<Vec<DeckSummary>> {
        self.0.store.borrow().list_decks(cards_core::time::now_ms())
    }

    /// The problems met while opening that did not stop it. Empty after
    /// the first call.
    pub fn take_open_warnings(&self) -> Vec<OpenWarning> {
        self.0.store.borrow_mut().take_open_warnings()
    }
}

/// The toast text for a warning from opening the database.
pub fn warning_message(warning: &OpenWarning) -> String {
    match warning {
        OpenWarning::DailyBackupFailed(err) => format!("Today’s backup could not be made: {err}"),
        OpenWarning::PruneFailed(err) => format!("Old backups could not be removed: {err}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn warnings_name_what_failed_and_why() {
        let err = || cards_core::Error::NotADirectory(PathBuf::from("/data/backups"));
        let text = warning_message(&OpenWarning::DailyBackupFailed(err()));
        assert!(text.starts_with("Today’s backup could not be made: "));
        assert!(text.contains("/data/backups"));
        let text = warning_message(&OpenWarning::PruneFailed(err()));
        assert!(text.starts_with("Old backups could not be removed: "));
    }
}
