//! The page shown when the database must be replaced by a backup: why,
//! the backups newest first, and a confirmed "Restore" for each. With no
//! backup to restore, a confirmed "Start Fresh" moves the database aside
//! and starts an empty one (ADR 0016). Layout in `data/recovery_page.blp`,
//! `data/restore_dialog.blp` and `data/start_fresh_dialog.blp`.

use std::path::PathBuf;
use std::rc::Rc;

use adw::prelude::*;
use adw::subclass::prelude::*;
use cards_core::storage::{BackupInfo, Restored};
use gtk::{gio, glib};

use crate::startup::{RecoveryReason, data_folder_line};
use crate::state::AppState;
use crate::ui::callback::Callback;
use crate::ui::dialog;
use crate::ui::toast::{persistent_toast, show_toast_from};

/// How the page got an open database.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Recovered {
    /// A backup was restored.
    Backup,
    /// The old database was moved aside and an empty one started.
    Fresh,
}

/// Called with the data directory, the new store and how it was made.
type OnRestored = dyn Fn(PathBuf, Restored, Recovered);

mod imp {
    use std::cell::{Cell, RefCell};
    use std::path::PathBuf;

    use adw::prelude::*;
    use adw::subclass::prelude::*;
    use gtk::glib;

    use super::OnRestored;
    use crate::startup::RecoveryReason;
    use crate::ui::callback::Callback;

    #[derive(Debug, Default, gtk::CompositeTemplate)]
    #[template(resource = "/io/github/rsmirke/Cards/recovery_page.ui")]
    pub struct RecoveryPage {
        #[template_child]
        pub status: TemplateChild<adw::StatusPage>,
        #[template_child]
        pub backup_list: TemplateChild<gtk::ListBox>,
        #[template_child]
        pub start_fresh_button: TemplateChild<gtk::Button>,
        #[template_child]
        pub path_label: TemplateChild<gtk::Label>,
        pub reason: Cell<Option<RecoveryReason>>,
        pub data_dir: RefCell<Option<PathBuf>>,
        pub on_restored: RefCell<Option<Callback<OnRestored>>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for RecoveryPage {
        const NAME: &'static str = "CardsRecoveryPage";
        type Type = super::RecoveryPage;
        type ParentType = adw::Bin;

        fn class_init(klass: &mut Self::Class) {
            klass.bind_template();
        }

        fn instance_init(obj: &glib::subclass::InitializingObject<Self>) {
            obj.init_template();
        }
    }

    impl ObjectImpl for RecoveryPage {
        fn constructed(&self) {
            self.parent_constructed();
            self.start_fresh_button.connect_clicked(glib::clone!(
                #[weak(rename_to = page)]
                self.obj(),
                move |_| page.confirm_start_fresh()
            ));
        }
    }
    impl WidgetImpl for RecoveryPage {}
    impl BinImpl for RecoveryPage {}
}

glib::wrapper! {
    pub struct RecoveryPage(ObjectSubclass<imp::RecoveryPage>)
        @extends adw::Bin, gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}

impl RecoveryPage {
    /// Explains `reason`, lists the backups in `data_dir`, and calls
    /// `on_restored` once a restore or a fresh start succeeds.
    pub fn show(
        &self,
        reason: RecoveryReason,
        data_dir: PathBuf,
        on_restored: impl Fn(PathBuf, Restored, Recovered) + 'static,
    ) {
        let imp = self.imp();
        imp.reason.set(Some(reason));
        imp.path_label.set_label(&data_folder_line(&data_dir));
        imp.data_dir.replace(Some(data_dir));
        imp.on_restored
            .replace(Some(Callback::new(Rc::new(on_restored))));
        self.reload();
    }

    /// Rebuilds the backup list and the explanation under the title.
    fn reload(&self) {
        let imp = self.imp();
        imp.backup_list.remove_all();
        imp.backup_list.set_sensitive(true);
        imp.start_fresh_button.set_visible(false);
        imp.start_fresh_button.set_sensitive(true);
        let Some(dir) = imp.data_dir.borrow().clone() else {
            return;
        };
        // The reason can change after a failed restore or start; the title
        // follows it.
        if let Some(reason) = imp.reason.get() {
            imp.status.set_title(reason.title());
        }
        let mut description = imp
            .reason
            .get()
            .map(RecoveryReason::description)
            .unwrap_or_default()
            .to_owned();
        match AppState::list_backups(&dir) {
            Ok(backups) if backups.is_empty() => {
                if offers_start_fresh(imp.reason.get()) {
                    imp.start_fresh_button.set_visible(true);
                    description.push_str(
                        "\n\nNo backups were found, so there is nothing to restore. You can \
                         start with an empty database; the current files are kept in the data \
                         folder, not deleted. Do not copy files there by hand.",
                    );
                } else {
                    description.push_str(
                        "\n\nNo backups were found, so there is nothing to restore. Leave the \
                         files in the data folder as they are and do not copy files there by \
                         hand.",
                    );
                }
            }
            Ok(backups) => {
                for backup in backups {
                    imp.backup_list.append(&self.backup_row(backup));
                }
            }
            Err(err) => {
                tracing::error!("cannot list backups: {err}");
                description.push_str("\n\nThe backups could not be listed: ");
                description.push_str(&err.to_string());
            }
        }
        imp.backup_list
            .set_visible(imp.backup_list.first_child().is_some());
        // The description is Pango markup; the error text and paths are not.
        imp.status
            .set_description(Some(&glib::markup_escape_text(&description)));
    }

    fn backup_row(&self, backup: BackupInfo) -> adw::ActionRow {
        let when = format_backup_time(backup.created_ms());
        let row = adw::ActionRow::builder()
            .title(when.as_str())
            .subtitle(glib::format_size(backup.size_bytes()).as_str())
            .use_markup(false)
            .build();
        let button = gtk::Button::builder()
            .label("_Restore")
            .use_underline(true)
            .valign(gtk::Align::Center)
            .build();
        // Every row has a "Restore" button; say which backup it restores.
        button.update_property(&[gtk::accessible::Property::Label(&format!(
            "Restore backup from {when}"
        ))]);
        button.connect_clicked(glib::clone!(
            #[weak(rename_to = page)]
            self,
            move |button| page.confirm_restore(button, backup.clone(), &when)
        ));
        row.add_suffix(&button);
        row
    }

    fn confirm_restore(&self, button: &gtk::Button, backup: BackupInfo, when: &str) {
        let Some(dialog) = dialog::alert("restore_dialog.ui", "restore_dialog") else {
            return;
        };
        dialog.set_heading(Some(&format!("Restore Backup From {when}?")));
        dialog.set_body(
            "Changes made after this backup will not be in the restored database. \
             The current database is kept in the data folder, not deleted.",
        );
        glib::spawn_future_local(glib::clone!(
            #[weak(rename_to = page)]
            self,
            #[weak]
            button,
            async move {
                if dialog.choose_future(Some(&page)).await == "restore" {
                    page.restore(&button, backup).await;
                }
            }
        ));
    }

    /// Restores on a worker thread: it copies and checks a whole database.
    async fn restore(&self, button: &gtk::Button, backup: BackupInfo) {
        let imp = self.imp();
        let Some(dir) = imp.data_dir.borrow().clone() else {
            return;
        };
        imp.backup_list.set_sensitive(false);
        button.set_child(Some(&adw::Spinner::new()));
        // Keeps the app running if the window is closed mid-restore, so the
        // restore finishes instead of being cut off.
        let _hold = self
            .root()
            .and_downcast::<gtk::Window>()
            .and_then(|window| window.application())
            .map(|app| app.hold());

        let worker_dir = dir.clone();
        let result =
            gio::spawn_blocking(move || AppState::restore_unopened(&worker_dir, &backup)).await;

        let message = match result {
            Ok(Ok(restored)) => {
                // Clone the callback out so no borrow is held while it runs.
                let handler = imp.on_restored.borrow().as_ref().map(Callback::get);
                if let Some(handler) = handler {
                    handler(dir, restored, Recovered::Backup);
                }
                return;
            }
            Ok(Err(cards_core::Error::RestoreRefused { source, .. })) => {
                tracing::warn!("backup refused: {source}");
                format!("This backup cannot be used: {source}")
            }
            Ok(Err(err)) => {
                tracing::error!("restore failed: {err}");
                // The old database may already be moved aside; say what is
                // now true on disk. Restoring again finishes it (ADR 0007).
                imp.reason.set(Some(RecoveryReason::InterruptedRestore));
                format!("Restore failed: {err}")
            }
            Err(_) => {
                tracing::error!("restore worker panicked");
                imp.reason.set(Some(RecoveryReason::InterruptedRestore));
                "Restore stopped unexpectedly".to_owned()
            }
        };
        self.reload();
        show_toast_from(self, persistent_toast(&message));
    }
}

impl RecoveryPage {
    fn confirm_start_fresh(&self) {
        let Some(dialog) = dialog::alert("start_fresh_dialog.ui", "start_fresh_dialog") else {
            return;
        };
        glib::spawn_future_local(glib::clone!(
            #[weak(rename_to = page)]
            self,
            async move {
                if dialog.choose_future(Some(&page)).await == "start" {
                    page.start_fresh().await;
                }
            }
        ));
    }

    /// Moves the database aside and starts an empty one, on a worker
    /// thread like a restore.
    async fn start_fresh(&self) {
        let imp = self.imp();
        let Some(dir) = imp.data_dir.borrow().clone() else {
            return;
        };
        imp.start_fresh_button.set_sensitive(false);
        // Keeps the app running if the window is closed meanwhile.
        let _hold = self
            .root()
            .and_downcast::<gtk::Window>()
            .and_then(|window| window.application())
            .map(|app| app.hold());

        let worker_dir = dir.clone();
        let result = gio::spawn_blocking(move || AppState::start_fresh_unopened(&worker_dir)).await;

        let message = match result {
            Ok(Ok(fresh)) => {
                let handler = imp.on_restored.borrow().as_ref().map(Callback::get);
                if let Some(handler) = handler {
                    handler(dir, fresh, Recovered::Fresh);
                }
                return;
            }
            Ok(Err(cards_core::Error::InterruptedRestore(_))) => {
                tracing::warn!("start fresh refused: a validated restore copy is waiting");
                imp.reason.set(Some(RecoveryReason::InterruptedRestore));
                "A restore did not finish. Its copy is kept; restore a backup to continue."
                    .to_owned()
            }
            Ok(Err(cards_core::Error::BackupsAvailable(_))) => {
                // A backup appeared since the list was shown; reloading
                // lists it.
                tracing::warn!("start fresh refused: backups exist");
                "Backups were found. Restore one of them instead.".to_owned()
            }
            Ok(Err(err)) => {
                tracing::error!("start fresh failed: {err}");
                format!("Starting fresh failed: {err}")
            }
            Err(_) => {
                tracing::error!("start fresh worker panicked");
                "Starting fresh stopped unexpectedly".to_owned()
            }
        };
        self.reload();
        show_toast_from(self, persistent_toast(&message));
    }
}

/// Whether the page offers Start Fresh when there is no backup to restore:
/// never while an interrupted restore's validated copy waits (that copy is
/// the user's data; ADR 0016). With backups it is never offered, and core
/// refuses it too.
fn offers_start_fresh(reason: Option<RecoveryReason>) -> bool {
    reason.is_some_and(|r| r != RecoveryReason::InterruptedRestore)
}

/// Local date and time of a backup, in the user's locale format.
fn format_backup_time(created_ms: i64) -> String {
    glib::DateTime::from_unix_local(created_ms.div_euclid(1000))
        .and_then(|time| time.format("%x %X"))
        .map_or_else(|_| "Unknown date".to_owned(), String::from)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn start_fresh_is_offered_for_every_reason_with_a_file_to_move() {
        for reason in [
            RecoveryReason::Damaged,
            RecoveryReason::NotCards,
            RecoveryReason::MissingFile,
        ] {
            assert!(offers_start_fresh(Some(reason)), "{reason:?}");
        }
    }

    #[test]
    fn start_fresh_never_hides_a_waiting_restore_copy() {
        assert!(!offers_start_fresh(Some(
            RecoveryReason::InterruptedRestore
        )));
        assert!(!offers_start_fresh(None));
    }
}
