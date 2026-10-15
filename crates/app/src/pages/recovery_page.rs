//! The page shown when the database must be replaced by a backup: why,
//! the backups newest first, and a confirmed "Restore" for each. Layout in
//! `data/recovery_page.blp` and `data/restore_dialog.blp`.

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

/// Called with the data directory and the restored store.
type OnRestored = dyn Fn(PathBuf, Restored);

mod imp {
    use std::cell::{Cell, RefCell};
    use std::path::PathBuf;

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

    impl ObjectImpl for RecoveryPage {}
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
    /// `on_restored` once a restore succeeds.
    pub fn show(
        &self,
        reason: RecoveryReason,
        data_dir: PathBuf,
        on_restored: impl Fn(PathBuf, Restored) + 'static,
    ) {
        let imp = self.imp();
        imp.reason.set(Some(reason));
        imp.status.set_title(reason.title());
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
        let Some(dir) = imp.data_dir.borrow().clone() else {
            return;
        };
        let mut description = imp
            .reason
            .get()
            .map(RecoveryReason::description)
            .unwrap_or_default()
            .to_owned();
        match AppState::list_backups(&dir) {
            Ok(backups) if backups.is_empty() => {
                description.push_str(
                    "\n\nNo backups were found, so there is nothing to restore. Leave the files \
                     in the data folder as they are and do not copy files there by hand.",
                );
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
                    handler(dir, restored);
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

/// Local date and time of a backup, in the user's locale format.
fn format_backup_time(created_ms: i64) -> String {
    glib::DateTime::from_unix_local(created_ms.div_euclid(1000))
        .and_then(|time| time.format("%x %X"))
        .map_or_else(|_| "Unknown date".to_owned(), String::from)
}
