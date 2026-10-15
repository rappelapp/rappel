//! Main application window. Layout lives in `data/window.blp`.

use std::path::{Path, PathBuf};

use adw::subclass::prelude::*;
use cards_core::storage::Restored;
use gtk::{gio, glib};

use crate::startup::Problem;
use crate::state::{AppState, warning_message};

mod imp {
    use std::cell::OnceCell;

    use adw::subclass::prelude::*;
    use gtk::glib;
    use gtk::prelude::*;

    use crate::pages::RecoveryPage;
    use crate::state::AppState;

    #[derive(Debug, Default, gtk::CompositeTemplate)]
    #[template(resource = "/io/github/rsmirke/Cards/window.ui")]
    pub struct CardsWindow {
        #[template_child]
        pub toast_overlay: TemplateChild<adw::ToastOverlay>,
        #[template_child]
        pub stack: TemplateChild<gtk::Stack>,
        #[template_child]
        pub main_page: TemplateChild<adw::StatusPage>,
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
    }

    #[glib::object_subclass]
    impl ObjectSubclass for CardsWindow {
        const NAME: &'static str = "CardsWindow";
        type Type = super::CardsWindow;
        type ParentType = adw::ApplicationWindow;

        fn class_init(klass: &mut Self::Class) {
            // The template uses this type, so it must be registered first.
            RecoveryPage::ensure_type();
            klass.bind_template();
        }

        fn instance_init(obj: &glib::subclass::InitializingObject<Self>) {
            obj.init_template();
        }
    }

    impl ObjectImpl for CardsWindow {}
    impl WidgetImpl for CardsWindow {}
    impl WindowImpl for CardsWindow {}
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
        match state.list_decks() {
            Ok(decks) if decks.is_empty() => {}
            Ok(decks) => {
                // Placeholder until the deck list exists.
                let title = match decks.len() {
                    1 => "1 Deck".to_owned(),
                    n => format!("{n} Decks"),
                };
                imp.main_page.set_title(&title);
                imp.main_page
                    .set_description(Some("Deck features are not available yet"));
            }
            Err(err) => {
                tracing::error!("cannot list decks: {err}");
                self.show_problem(Problem::Failed {
                    message: "The decks could not be read.".to_owned(),
                    details: err.to_string(),
                    data_dir: Some(state.data_dir().to_path_buf()),
                });
                return;
            }
        }
        if imp.state.set(state).is_err() {
            // Only reachable if startup and a restore both succeeded, but
            // the restore page is only shown when startup failed.
            tracing::error!("the window already has an open database");
            return;
        }
        tracing::debug!("showing the main page");
        imp.stack.set_visible_child_name("main");
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
                        move |dir, restored| window.restored(dir, restored)
                    ),
                );
                imp.stack.set_visible_child_name("recovery");
            }
            Problem::TooNew {
                found,
                supported,
                data_dir,
            } => {
                imp.too_new_page.set_description(Some(&escape(&format!(
                    "This database was made by a newer version of Cards (database version \
                     {found}; this version supports up to {supported}). Update Cards to open \
                     it. The database has not been changed."
                ))));
                imp.too_new_path.set_label(&data_folder_line(&data_dir));
                imp.stack.set_visible_child_name("too-new");
            }
            Problem::Failed {
                message,
                details,
                data_dir,
            } => {
                imp.error_page.set_description(Some(&escape(&message)));
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

    fn restored(&self, data_dir: PathBuf, restored: Restored) {
        let message = match &restored.replaced {
            Some(path) => format!(
                "Backup restored. The previous database was kept as {}",
                path.display()
            ),
            None => "Backup restored".to_owned(),
        };
        self.show_main(AppState::new(data_dir, restored.store));
        self.add_toast(persistent_toast(&message));
    }
}

/// A toast with plain text (paths and errors may contain `<` or `&`) that
/// stays until dismissed, so the user cannot miss it.
pub fn persistent_toast(text: &str) -> adw::Toast {
    adw::Toast::builder()
        .title(text)
        .use_markup(false)
        .timeout(0)
        .build()
}

/// Escapes text for a markup property such as `AdwStatusPage:description`.
fn escape(text: &str) -> glib::GString {
    glib::markup_escape_text(text)
}

fn data_folder_line(dir: &Path) -> String {
    format!("Data folder: {}", dir.display())
}
