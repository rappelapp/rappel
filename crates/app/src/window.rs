//! Main application window. Layout lives in `data/window.blp`.

use std::path::{Path, PathBuf};

use adw::prelude::*;
use adw::subclass::prelude::*;
use cards_core::domain::Card;
use cards_core::storage::Restored;
use gtk::{gio, glib};

use crate::dialogs::card_editor::CardEditor;
use crate::interchange;
use crate::pages::{DeckPage, DecksPage, Recovered};
use crate::startup::Problem;
use crate::startup::data_folder_line;
use crate::state::AppState;
use crate::ui::toast::{persistent_toast, warning_message};

/// The two jobs of the primary menu.
#[derive(Debug, Clone, Copy)]
enum Transfer {
    Export,
    Import,
}

/// The window actions that start a [`Transfer`].
const TRANSFER_ACTIONS: [&str; 2] = ["win.export", "win.import"];

mod imp {
    use std::cell::{Cell, OnceCell};

    use adw::subclass::prelude::*;
    use gtk::glib;
    use gtk::prelude::*;

    use crate::pages::{DeckPage, DecksPage, RecoveryPage};
    use crate::state::AppState;

    #[derive(Debug, Default, gtk::CompositeTemplate)]
    #[template(resource = "/io/github/rsmirke/Cards/window.ui")]
    pub struct CardsWindow {
        #[template_child]
        pub toast_overlay: TemplateChild<adw::ToastOverlay>,
        #[template_child]
        pub stack: TemplateChild<gtk::Stack>,
        #[template_child]
        pub navigation_view: TemplateChild<adw::NavigationView>,
        #[template_child]
        pub decks_page: TemplateChild<DecksPage>,
        #[template_child]
        pub recovery_page: TemplateChild<RecoveryPage>,
        #[template_child]
        pub too_new_page: TemplateChild<adw::StatusPage>,
        #[template_child]
        pub too_new_path: TemplateChild<gtk::Label>,
        #[template_child]
        pub error_page: TemplateChild<adw::StatusPage>,
        #[template_child]
        pub error_details: TemplateChild<gtk::Label>,
        /// Set once, when the database is open (at startup or after a
        /// restore). Pages and dialogs get a clone from here.
        pub state: OnceCell<AppState>,
        /// An export or import is running; another one cannot start.
        pub transferring: Cell<bool>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for CardsWindow {
        const NAME: &'static str = "CardsWindow";
        type Type = super::CardsWindow;
        type ParentType = adw::ApplicationWindow;

        fn class_init(klass: &mut Self::Class) {
            // The template uses these types, so they must be registered first.
            DecksPage::ensure_type();
            RecoveryPage::ensure_type();
            klass.bind_template();
            // Enabled once the deck list is shown.
            klass.install_action("win.new-deck", None, |window, _, _| {
                window.imp().decks_page.new_deck();
            });
            // Ctrl+N (set in main.rs): a new card on a deck page, a new
            // deck on the deck list, nothing while studying.
            klass.install_action("win.new", None, |window, _, _| {
                let imp = window.imp();
                let Some(page) = imp.navigation_view.visible_page() else {
                    return;
                };
                if let Some(deck_page) = page.downcast_ref::<DeckPage>() {
                    deck_page.new_card();
                } else if page.is::<DecksPage>() {
                    imp.decks_page.new_deck();
                }
            });
            // The primary menu; enabled on the main page while no export
            // or import runs.
            klass.install_action("win.export", None, |window, _, _| {
                window.start_transfer(super::Transfer::Export);
            });
            klass.install_action("win.import", None, |window, _, _| {
                window.start_transfer(super::Transfer::Import);
            });
        }

        fn instance_init(obj: &glib::subclass::InitializingObject<Self>) {
            obj.init_template();
        }
    }

    impl ObjectImpl for CardsWindow {
        fn constructed(&self) {
            self.parent_constructed();
            let window = self.obj();
            window.action_set_enabled("win.new-deck", false);
            window.action_set_enabled("win.new", false);
            for action in super::TRANSFER_ACTIONS {
                window.action_set_enabled(action, false);
            }
        }
    }
    impl WidgetImpl for CardsWindow {}
    impl WindowImpl for CardsWindow {
        /// Closing the window with unsaved text in a card editor asks the
        /// editor's "Discard Changes?" first, as Escape does.
        fn close_request(&self) -> glib::Propagation {
            let window = self.obj();
            let Some(editor) = window.unsaved_editor() else {
                return self.parent_close_request();
            };
            glib::spawn_future_local(glib::clone!(
                #[weak]
                window,
                #[weak]
                editor,
                async move {
                    if editor.confirm_discard().await {
                        editor.discard();
                        window.close();
                    }
                }
            ));
            glib::Propagation::Stop
        }
    }
    impl ApplicationWindowImpl for CardsWindow {}
    impl AdwApplicationWindowImpl for CardsWindow {}
}

glib::wrapper! {
    pub struct CardsWindow(ObjectSubclass<imp::CardsWindow>)
        @extends adw::ApplicationWindow, gtk::ApplicationWindow, gtk::Window, gtk::Widget,
        @implements gio::ActionGroup, gio::ActionMap, gtk::Accessible, gtk::Buildable,
                    gtk::ConstraintTarget, gtk::Native, gtk::Root, gtk::ShortcutManager;
}

impl CardsWindow {
    pub fn new(app: &adw::Application) -> Self {
        glib::Object::builder().property("application", app).build()
    }

    /// An open card editor holding unsaved text, if any. Every open dialog
    /// is searched, so it is found even under its "Discard Changes?"
    /// question.
    fn unsaved_editor(&self) -> Option<CardEditor> {
        let dialogs = self.dialogs();
        (0..dialogs.n_items())
            .filter_map(|i| dialogs.item(i).and_downcast::<CardEditor>())
            .find(CardEditor::has_unsaved_changes)
    }

    /// Shows `toast` over the window content.
    pub fn add_toast(&self, toast: adw::Toast) {
        self.imp().toast_overlay.add_toast(toast);
    }

    /// Takes the open database and shows the main page, with a toast for
    /// each problem met while opening it.
    pub fn show_main(&self, state: AppState) {
        let imp = self.imp();
        for warning in state.take_open_warnings() {
            self.add_toast(persistent_toast(&warning_message(&warning)));
        }
        // Checked before keeping the state, so a failure leaves the window
        // without one (and a later retry could still call this).
        if let Err(err) = state.list_decks() {
            self.decks_unreadable(&err, state.data_dir());
            return;
        }
        if imp.state.set(state).is_err() {
            // Only reachable if startup and a restore both succeeded, but
            // the restore page is only shown when startup failed.
            tracing::error!("the window already has an open database");
            return;
        }
        if let Some(state) = imp.state.get()
            && let Err(err) = imp.decks_page.set_state(state.clone())
        {
            // Never show "No Decks Yet" for decks that could not be read.
            self.decks_unreadable(&err, state.data_dir());
            return;
        }
        tracing::debug!("showing the main page");
        imp.stack.set_visible_child_name("main");
        self.action_set_enabled("win.new-deck", true);
        self.action_set_enabled("win.new", true);
        for action in TRANSFER_ACTIONS {
            self.action_set_enabled(action, true);
        }
    }

    /// Runs an export or import unless one is already running. Its actions
    /// are disabled meanwhile. `interchange` holds the application around
    /// each worker, so closing the window lets the work finish.
    fn start_transfer(&self, transfer: Transfer) {
        let imp = self.imp();
        let Some(state) = imp.state.get().cloned() else {
            return;
        };
        if imp.transferring.replace(true) {
            return;
        }
        for action in TRANSFER_ACTIONS {
            self.action_set_enabled(action, false);
        }
        glib::spawn_future_local(glib::clone!(
            #[weak(rename_to = window)]
            self,
            async move {
                match transfer {
                    Transfer::Export => interchange::export(&window, &state).await,
                    Transfer::Import => interchange::import(&window, &state).await,
                }
                window.imp().transferring.set(false);
                for action in TRANSFER_ACTIONS {
                    window.action_set_enabled(action, true);
                }
            }
        ));
    }

    /// After an import: reads the decks and every open deck page's cards
    /// again. A study session keeps the cards it started with.
    pub fn data_changed(&self) {
        let stack = self.imp().navigation_view.navigation_stack();
        for page in (0..stack.n_items()).filter_map(|i| stack.item(i)) {
            if let Some(decks) = page.downcast_ref::<DecksPage>() {
                decks.refresh();
            } else if let Some(deck) = page.downcast_ref::<DeckPage>() {
                deck.reload();
            }
        }
    }

    /// Shows the error page for decks that could not be read in `data_dir`.
    fn decks_unreadable(&self, err: &cards_core::Error, data_dir: &Path) {
        tracing::error!("cannot list decks: {err}");
        self.show_problem(Problem::Failed {
            message: "The decks could not be read.".to_owned(),
            details: err.to_string(),
            data_dir: Some(data_dir.to_path_buf()),
        });
    }

    /// After an Undo from a toast brought `card` back: shows it on the
    /// visible deck page if that is its deck, and updates the deck counts.
    pub fn card_restored(&self, card: &Card) {
        let imp = self.imp();
        if let Some(page) = imp
            .navigation_view
            .visible_page()
            .and_downcast::<DeckPage>()
        {
            page.restored(card);
        }
        imp.decks_page.refresh();
    }

    /// Shows the full-window page that explains `problem`.
    pub fn show_problem(&self, problem: Problem) {
        let imp = self.imp();
        tracing::debug!(?problem, "showing a startup problem page");
        match problem {
            Problem::Recovery { reason, data_dir } => {
                imp.recovery_page.show(
                    reason,
                    data_dir,
                    glib::clone!(
                        #[weak(rename_to = window)]
                        self,
                        move |dir, restored, how| window.restored(dir, restored, how)
                    ),
                );
                imp.stack.set_visible_child_name("recovery");
            }
            Problem::TooNew {
                found,
                supported,
                data_dir,
            } => {
                imp.too_new_page
                    .set_description(Some(&glib::markup_escape_text(&format!(
                        "This database was made by a newer version of Cards (database version \
                         {found}; this version supports up to {supported}). Update Cards to \
                         open it. The database has not been changed."
                    ))));
                imp.too_new_path.set_label(&data_folder_line(&data_dir));
                imp.stack.set_visible_child_name("too-new");
            }
            Problem::Failed {
                message,
                details,
                data_dir,
            } => {
                imp.error_page
                    .set_description(Some(&glib::markup_escape_text(&message)));
                let mut text = details;
                if let Some(dir) = data_dir {
                    text.push_str("\n\n");
                    text.push_str(&data_folder_line(&dir));
                }
                imp.error_details.set_label(&text);
                imp.stack.set_visible_child_name("error");
            }
        }
    }

    fn restored(&self, data_dir: PathBuf, restored: Restored, how: Recovered) {
        let message = recovered_message(how, restored.replaced.as_deref());
        self.show_main(AppState::new(data_dir, restored.store));
        self.add_toast(persistent_toast(&message));
    }
}

/// The toast after the recovery page got a database: what happened, and
/// where the previous database was kept.
fn recovered_message(how: Recovered, replaced: Option<&Path>) -> String {
    let done = match how {
        Recovered::Backup => "Backup restored",
        Recovered::Fresh => "Started with an empty database",
    };
    match replaced {
        Some(path) => format!(
            "{done}. The previous database was kept as {}",
            path.display()
        ),
        None => done.to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recovery_toasts_say_what_happened_and_where_the_old_file_is() {
        let kept = Path::new("/data/cards.sqlite3.replaced-20261015-120000");
        assert_eq!(
            recovered_message(Recovered::Backup, Some(kept)),
            "Backup restored. The previous database was kept as \
             /data/cards.sqlite3.replaced-20261015-120000"
        );
        assert_eq!(
            recovered_message(Recovered::Backup, None),
            "Backup restored"
        );
        assert_eq!(
            recovered_message(Recovered::Fresh, Some(kept)),
            "Started with an empty database. The previous database was kept as \
             /data/cards.sqlite3.replaced-20261015-120000"
        );
        assert_eq!(
            recovered_message(Recovered::Fresh, None),
            "Started with an empty database"
        );
    }
}
