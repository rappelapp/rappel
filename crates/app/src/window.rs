//! Main application window. Layout lives in `data/window.blp`.

use std::path::{Path, PathBuf};

use adw::prelude::*;
use adw::subclass::prelude::*;
use gtk::{gio, glib};
use rappel_core::domain::{Card, SettingKey};
use rappel_core::storage::Restored;

use crate::dialogs::card_editor::CardEditor;
use crate::i18n::{fill, gettext};
use crate::pages::{ArchivedPage, BrowserPage, DeckPage, DecksPage, Recovered, ReviewPage};
use crate::startup::{Problem, data_folder_line, setting_reset_text, too_new_text};
use crate::state::AppState;
use crate::transfer;
use crate::ui::toast::{info_toast, persistent_toast, warning_message};
use crate::ui::{nav, worker};
use crate::window_state;

/// The two jobs of the primary menu.
#[derive(Debug, Clone, Copy)]
enum Transfer {
    Export,
    Import,
}

/// The window actions that start a [`Transfer`].
const TRANSFER_ACTIONS: [&str; 2] = ["win.export", "win.import"];

/// The icons of the "too new" and "could not start" pages, as in
/// `data/window.blp`, given back after a short window took them off.
const TOO_NEW_ICON: &str = "software-update-available-symbolic";
const ERROR_ICON: &str = "dialog-error-symbolic";

mod imp {
    use std::cell::{Cell, OnceCell};

    use adw::subclass::prelude::*;
    use gtk::glib;
    use gtk::prelude::*;

    use crate::pages::{DeckPage, DecksPage, RecoveryPage, SettingPage};
    use crate::state::AppState;

    #[derive(Debug, Default, gtk::CompositeTemplate)]
    #[template(resource = "/io/github/rappelapp/Rappel/window.ui")]
    pub struct RappelWindow {
        #[template_child]
        pub toast_overlay: TemplateChild<adw::ToastOverlay>,
        #[template_child]
        pub stack: TemplateChild<adw::ViewStack>,
        #[template_child]
        pub navigation_view: TemplateChild<adw::NavigationView>,
        #[template_child]
        pub decks_page: TemplateChild<DecksPage>,
        #[template_child]
        pub recovery_page: TemplateChild<RecoveryPage>,
        #[template_child]
        pub setting_page: TemplateChild<SettingPage>,
        #[template_child]
        pub too_new_page: TemplateChild<adw::StatusPage>,
        #[template_child]
        pub too_new_path: TemplateChild<gtk::Label>,
        #[template_child]
        pub error_page: TemplateChild<adw::StatusPage>,
        #[template_child]
        pub error_details: TemplateChild<gtk::Label>,
        #[template_child]
        pub too_new_short: TemplateChild<adw::Breakpoint>,
        #[template_child]
        pub error_short: TemplateChild<adw::Breakpoint>,
        /// Set once, when the database is open (at startup or after a
        /// restore). Pages and dialogs get a clone from here.
        pub state: OnceCell<AppState>,
        /// An export or import is running; another one cannot start.
        pub transferring: Cell<bool>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for RappelWindow {
        const NAME: &'static str = "RappelWindow";
        type Type = super::RappelWindow;
        type ParentType = adw::ApplicationWindow;

        fn class_init(klass: &mut Self::Class) {
            // The template uses these types, so they must be registered first.
            DecksPage::ensure_type();
            RecoveryPage::ensure_type();
            SettingPage::ensure_type();
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
            // Ctrl+F (set in main.rs) and the deck list's search button: the
            // All Cards page, filtered to the deck when asked from a deck page.
            klass.install_action("win.search", None, |window, _, _| {
                window.search();
            });
            // The primary menu; enabled on the main page while no export
            // or import runs.
            klass.install_action("win.export", None, |window, _, _| {
                window.start_transfer(super::Transfer::Export);
            });
            klass.install_action("win.import", None, |window, _, _| {
                window.start_transfer(super::Transfer::Import);
            });
            // Ctrl+, (set in main.rs); enabled once the deck list is shown.
            klass.install_action("win.preferences", None, |window, _, _| {
                if let Some(state) = window.imp().state.get() {
                    crate::dialogs::preferences::show(window, state);
                }
            });
            // The primary menu. Needs no database, so always enabled.
            klass.install_action("win.about", None, |window, _, _| {
                crate::dialogs::about::show(window);
            });
        }

        fn instance_init(obj: &glib::subclass::InitializingObject<Self>) {
            obj.init_template();
        }
    }

    impl ObjectImpl for RappelWindow {
        fn constructed(&self) {
            self.parent_constructed();
            let window = self.obj();
            window.action_set_enabled("win.new-deck", false);
            window.action_set_enabled("win.new", false);
            window.action_set_enabled("win.search", false);
            window.action_set_enabled("win.preferences", false);
            // The only focusable widgets of their pages.
            crate::ui::focus::unselect_on_first_focus(&self.too_new_path);
            crate::ui::focus::unselect_on_first_focus(&self.error_details);
            for action in super::TRANSFER_ACTIONS {
                window.action_set_enabled(action, false);
            }
            // The on-screen keyboard never covers the focused field
            // (ADR 0058).
            crate::ui::scroll::install(&*window);
            // The "too new" and "could not start" pages on a short window
            // (ADR 0058).
            crate::ui::breakpoint::compact_status_page(
                &self.too_new_short,
                &self.too_new_page,
                super::TOO_NEW_ICON,
            );
            crate::ui::breakpoint::compact_status_page(
                &self.error_short,
                &self.error_page,
                super::ERROR_ICON,
            );
        }
    }
    impl WidgetImpl for RappelWindow {}
    impl WindowImpl for RappelWindow {
        /// Closing the window with unsaved text in a card editor asks the
        /// editor's "Discard Changes?" first, as Escape does.
        fn close_request(&self) -> glib::Propagation {
            let window = self.obj();
            let Some(editor) = window.unsaved_editor() else {
                window.save_state();
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
    impl ApplicationWindowImpl for RappelWindow {}
    impl AdwApplicationWindowImpl for RappelWindow {}
}

glib::wrapper! {
    pub struct RappelWindow(ObjectSubclass<imp::RappelWindow>)
        @extends adw::ApplicationWindow, gtk::ApplicationWindow, gtk::Window, gtk::Widget,
        @implements gio::ActionGroup, gio::ActionMap, gtk::Accessible, gtk::Buildable,
                    gtk::ConstraintTarget, gtk::Native, gtk::Root, gtk::ShortcutManager;
}

impl RappelWindow {
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
        if let Err(err) = state.list_decks(rappel_core::time::now_ms()) {
            if let rappel_core::Error::DamagedSetting { key, .. } = err {
                // Damaged after the open checked it (another process): the
                // same page as at startup. The state is not kept, so the
                // reset opens the database on its own.
                self.show_problem(Problem::DamagedSetting {
                    key,
                    data_dir: state.data_dir().to_path_buf(),
                });
                return;
            }
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
        self.tidy_media_trash();
        // Only at startup, before the window is shown; not after a restore.
        if !self.is_mapped() {
            self.restore_state();
        }
        self.action_set_enabled("win.new-deck", true);
        self.action_set_enabled("win.new", true);
        self.action_set_enabled("win.search", true);
        self.action_set_enabled("win.preferences", true);
        for action in TRANSFER_ACTIONS {
            self.action_set_enabled(action, true);
        }
    }

    /// Brings back the media trash's files the database uses and purges
    /// the old ones nothing uses (ADR 0057), on a worker after the main
    /// page is shown, so it never holds the window back. At startup and
    /// after a restore. Only logged: the user asked for nothing.
    fn tidy_media_trash(&self) {
        let Some(data_dir) = self.imp().state.get().map(|s| s.data_dir().to_path_buf()) else {
            return;
        };
        glib::spawn_future_local(glib::clone!(
            #[weak(rename_to = window)]
            self,
            async move {
                let result = worker::run(&window, move || {
                    AppState::tidy_media_trash_on_worker(&data_dir)
                })
                .await;
                match result {
                    Ok(Ok(tidied)) => {
                        if tidied.blocked {
                            tracing::warn!(
                                "the media trash was not purged: a backup cannot be read"
                            );
                        }
                        tracing::debug!(?tidied, "media trash tidied");
                    }
                    Ok(Err(err)) => tracing::warn!("tidying the media trash failed: {err}"),
                    Err(_) => tracing::warn!("tidying the media trash stopped unexpectedly"),
                }
            }
        ));
    }

    /// Applies the size, maximised state and open deck saved when the
    /// window last closed (`window_state`). Called before the window is
    /// first presented.
    fn restore_state(&self) {
        let imp = self.imp();
        let Some(state) = imp.state.get() else {
            return;
        };
        let saved = window_state::load(state.data_dir());
        let (width, height) =
            window_state::fit_to_monitors(saved.width, saved.height, &monitor_sizes(self));
        self.set_default_size(width, height);
        if saved.maximized {
            // "It's permitted to call this function before showing a window,
            // in which case the window will be maximized when it appears
            // onscreen initially" (gtk_window_maximize docs).
            self.maximize();
        }
        if let Some(id) = saved.deck_id {
            // Straight to the deck page, without the slide from the list.
            let view = &imp.navigation_view;
            let animate = view.is_animate_transitions();
            view.set_animate_transitions(false);
            imp.decks_page.reopen(&id);
            view.set_animate_transitions(animate);
        }
    }

    /// The pages of the navigation stack, bottom first.
    fn pages(&self) -> Vec<adw::NavigationPage> {
        let stack = self.imp().navigation_view.navigation_stack();
        (0..stack.n_items())
            .filter_map(|i| stack.item(i).and_downcast())
            .collect()
    }

    /// Saves the size, maximised state and open deck for the next start.
    /// Only from the main page: a recovery or error page has no database
    /// and nothing to remember.
    fn save_state(&self) {
        let imp = self.imp();
        let Some(state) = imp.state.get() else {
            return;
        };
        if imp.stack.visible_child_name().as_deref() != Some("main") {
            return;
        }
        let deck_id = window_state::deck_to_reopen(
            self.pages()
                .iter()
                .map(|page| page.downcast_ref::<DeckPage>().and_then(DeckPage::deck_id)),
        );
        // The size the window has when not maximised: "the recommended way
        // for saving window state across restarts" (gtk_window_get_default_size
        // docs).
        let (width, height) = self.default_size();
        let saved = window_state::WindowState {
            width,
            height,
            maximized: self.is_maximized(),
            deck_id,
        };
        window_state::save(state.data_dir(), &saved);
    }

    /// Runs an export or import unless one is already running. Its actions
    /// are disabled meanwhile. `transfer` holds the application around
    /// each worker (`ui::worker`), so closing the window lets the work finish.
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
                    Transfer::Export => transfer::export(&window, &state).await,
                    Transfer::Import => transfer::import(&window, &state).await,
                }
                window.imp().transferring.set(false);
                for action in TRANSFER_ACTIONS {
                    window.action_set_enabled(action, true);
                }
            }
        ));
    }

    /// Opens All Cards (filtered to the deck of a deck page), or, if it is
    /// open, puts the keyboard in its search field. Not while studying, and
    /// not under an open dialog (the editor): the page would open behind it.
    fn search(&self) {
        let imp = self.imp();
        let Some(state) = imp.state.get() else {
            return;
        };
        if self.visible_dialog().is_some() {
            return;
        }
        let Some(page) = imp.navigation_view.visible_page() else {
            return;
        };
        let deck_id = if let Some(browser) = page.downcast_ref::<BrowserPage>() {
            browser.focus_search();
            return;
        } else if let Some(deck) = page.downcast_ref::<DeckPage>() {
            // Not while cards are being chosen, as on All Cards
            // (`BrowserPage::focus_search`): Cancel or Escape comes first.
            if deck.selecting() {
                return;
            }
            deck.deck_id()
        } else if page.is::<DecksPage>() {
            None
        } else {
            return;
        };
        // The visible page is not All Cards (see above); the helper keeps
        // the rule in one place.
        if let Err(err) = nav::push_unless_on_top(&imp.navigation_view, || {
            BrowserPage::new(state.clone(), deck_id.as_deref())
        }) {
            crate::ui::toast::show_error(
                self,
                "opening All Cards",
                // Translators: a toast: what is before ":" is its title, the
                // rest is behind Details. {error} is why, in English.
                &crate::ui::text::with_error(
                    &gettext("The cards could not be read: {error}"),
                    &err,
                ),
            );
        }
    }

    /// After a reset or its undo: the counts and the new cards of every open
    /// deck page, and the deck list.
    pub fn study_changed(&self) {
        let imp = self.imp();
        for page in self.pages() {
            if let Some(deck) = page.downcast_ref::<DeckPage>() {
                deck.update_study();
                deck.refresh_new_flags();
            } else if let Some(browser) = page.downcast_ref::<BrowserPage>() {
                // A deck page's Undo (suspend, reset) changed cards it lists.
                browser.requery();
            }
        }
        imp.decks_page.refresh();
    }

    /// After an import, or a change made on All Cards or the review page
    /// (suspend, delete and their Undo), or a deck archived or unarchived:
    /// reads the decks, the archived decks and every open deck page's cards
    /// again. A study session keeps the cards it started with.
    pub fn data_changed(&self) {
        for page in self.pages() {
            if let Some(decks) = page.downcast_ref::<DecksPage>() {
                decks.refresh();
            } else if let Some(archived) = page.downcast_ref::<ArchivedPage>() {
                archived.refresh();
            } else if let Some(deck) = page.downcast_ref::<DeckPage>() {
                deck.reload();
            } else if let Some(browser) = page.downcast_ref::<BrowserPage>() {
                browser.reload_all();
            }
        }
    }

    /// After a merge's Undo brought deck `source` back: every open deck
    /// page and All Cards read their cards again (the target lost the
    /// cards), and the deck list shows the deck again, focused if the list
    /// is showing (ADR 0066).
    pub fn deck_unmerged(&self, source: &str) {
        for page in self.pages() {
            if let Some(deck) = page.downcast_ref::<DeckPage>() {
                deck.reload();
            } else if let Some(browser) = page.downcast_ref::<BrowserPage>() {
                browser.reload_all();
            }
        }
        self.imp().decks_page.deck_restored(source);
    }

    /// After deck page `page`'s deck was merged (`done`):
    /// shows the deck list's "Decks merged" toast and goes back to the
    /// list, which focuses the row of the deck it went into once shown
    /// (ADR 0066: the merged deck has no page any more).
    pub fn leave_merged_deck(
        &self,
        page: &DeckPage,
        state: &AppState,
        done: crate::dialogs::merge_deck::Done,
    ) {
        let decks = &self.imp().decks_page;
        decks.focus_on_return(&done.target.id);
        decks.show_merged_toast(state, done);
        nav::pop(page);
    }

    /// After deck page `page`'s deck was archived: shows the deck list's
    /// "Deck archived" toast and goes back to the list, which focuses the
    /// row now at the deck's place once shown (ADR 0068: an archived deck
    /// has no page).
    pub fn leave_archived_deck(&self, page: &DeckPage, state: &AppState, id: &str) {
        self.deck_archived(id);
        let decks = &self.imp().decks_page;
        decks.focus_place_on_return(id);
        decks.show_archived_toast(state, id.to_owned());
        nav::pop(page);
    }

    /// Archives deck `id` again: the Undo of "Deck unarchived" (ADR 0068).
    /// As Archive does, a page of that deck (its deck page, and a study
    /// session or statistics above it) is left for the deck list, which
    /// focuses the row now at the deck's place; on the deck list the focus
    /// goes there at once, and on the Archived Decks page to the deck's
    /// Unarchive button. Every page reads its decks again.
    ///
    /// # Errors
    /// What [`AppState::archive_deck`] returns; nothing changed then.
    pub fn rearchive_deck(&self, state: &AppState, id: &str) -> rappel_core::Result<()> {
        let imp = self.imp();
        let decks = &imp.decks_page;
        let open = self.pages().iter().any(|page| {
            let deck = page
                .downcast_ref::<DeckPage>()
                .and_then(DeckPage::deck_id)
                .or_else(|| {
                    page.downcast_ref::<ReviewPage>()
                        .and_then(ReviewPage::deck_id)
                });
            deck.as_deref() == Some(id)
        });
        state.archive_deck(id)?;
        self.deck_archived(id);
        // Read while the row is still in the list: it leaves at the next
        // reload.
        decks.focus_place_on_return(id);
        if open {
            // `shown` focuses the row at the deck's place.
            imp.navigation_view.pop_to_page(&**decks);
        } else {
            decks.deck_left_list();
        }
        self.data_changed();
        if let Some(page) = imp
            .navigation_view
            .visible_page()
            .and_downcast::<ArchivedPage>()
        {
            page.focus_deck(id);
        }
        Ok(())
    }

    /// Deck `id` was just archived: an open tag session drops its cards
    /// (`ReviewPage::leave_deck`, ADR 0069). A page of the deck itself
    /// (its page, its own session) is left by the caller.
    pub fn deck_archived(&self, id: &str) {
        for page in self.pages() {
            if let Some(review) = page.downcast_ref::<ReviewPage>() {
                review.leave_deck(id);
            }
        }
    }

    /// Makes the deck list focus deck `id`'s row the next time it shows
    /// (the Archived Decks page going back after its last Unarchive).
    pub fn focus_deck_on_return(&self, id: &str) {
        self.imp().decks_page.focus_on_return(id);
    }

    /// Shows the error page for decks that could not be read in `data_dir`.
    fn decks_unreadable(&self, err: &rappel_core::Error, data_dir: &Path) {
        tracing::error!("cannot list decks: {err}");
        self.show_problem(Problem::Failed {
            message: gettext("The decks could not be read."),
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
        // An open All Cards page lists the card too.
        for page in self.pages() {
            if let Some(browser) = page.downcast_ref::<BrowserPage>() {
                browser.requery();
            }
        }
        imp.decks_page.refresh();
    }

    /// Shows the full-window page that explains `problem`.
    pub fn show_problem(&self, problem: Problem) {
        let imp = self.imp();
        tracing::debug!(?problem, "showing a startup problem page");
        match problem {
            Problem::Recovery { reason, data_dir } => {
                let dir = data_dir.clone();
                imp.recovery_page.set_on_too_new(glib::clone!(
                    #[weak(rename_to = window)]
                    self,
                    move |found, supported| window.show_problem(Problem::TooNew {
                        found,
                        supported,
                        data_dir: dir.clone(),
                    })
                ));
                imp.recovery_page.set_on_damaged_setting(glib::clone!(
                    #[weak(rename_to = window)]
                    self,
                    #[strong]
                    data_dir,
                    move |key, how, replaced, set_aside| {
                        window.show_problem(Problem::DamagedSetting {
                            key,
                            data_dir: data_dir.clone(),
                        });
                        // The files were moved all the same: say where.
                        window.add_toast(persistent_toast(&recovered_message(
                            how,
                            replaced.as_deref(),
                            set_aside.as_deref(),
                        )));
                    }
                ));
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
            Problem::DamagedSetting { key, data_dir } => {
                imp.setting_page.show(
                    key,
                    data_dir,
                    glib::clone!(
                        #[weak(rename_to = window)]
                        self,
                        move |dir, store, key, was_reset| {
                            window.setting_reset(dir, store, key, was_reset);
                        }
                    ),
                );
                imp.stack.set_visible_child_name("setting");
                // After the page is the visible one: its only action takes
                // the keyboard, as Start Fresh does on the recovery page.
                imp.setting_page.focus_action();
            }
            Problem::TooNew {
                found,
                supported,
                data_dir,
            } => {
                imp.too_new_page
                    .set_description(Some(&glib::markup_escape_text(&too_new_text(
                        found, supported,
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

    /// After the setting page reset the damaged setting: the main page, and
    /// a toast that says what was reset to what.
    fn setting_reset(
        &self,
        data_dir: PathBuf,
        store: rappel_core::storage::Store,
        key: SettingKey,
        was_reset: bool,
    ) {
        self.show_main(AppState::new(data_dir, store));
        self.add_toast(info_toast(&setting_reset_text(key, was_reset)));
    }

    fn restored(&self, data_dir: PathBuf, restored: Restored, how: Recovered) {
        let message = recovered_message(
            how,
            restored.replaced.as_deref(),
            restored.set_aside.as_deref(),
        );
        self.show_main(AppState::new(data_dir, restored.store));
        self.add_toast(persistent_toast(&message));
    }
}

/// The toast after the recovery page got a database: what happened, where
/// the previous database was kept, and where the copy an interrupted restore
/// had left waiting was kept (`set_aside`): Start Fresh moves one that could
/// not be used (ADR 0050, row 35), a restore sets any aside (ADR 0052).
fn recovered_message(how: Recovered, replaced: Option<&Path>, set_aside: Option<&Path>) -> String {
    // The first sentence is the toast's title when the paths make the text
    // too long for it, so it is short (`ui::toast::toast_text`).
    let (done, short) = match how {
        Recovered::Backup => (gettext("Backup restored"), gettext("Backup restored")),
        Recovered::Finished => (gettext("Restore finished"), gettext("Restore finished")),
        Recovered::Fresh => (
            gettext("Started with an empty database"),
            gettext("Started fresh"),
        ),
    };
    let mut message = match replaced {
        Some(path) => fill(
            // Translators: a toast: its first sentence is its title, the
            // rest is behind Details. {title} is "Backup restored" or the
            // like, {path} a file's path.
            &gettext("{title}. The previous database was kept as {path}"),
            &[("title", &short), ("path", &path.display().to_string())],
        ),
        None if set_aside.is_some() => short,
        None => done,
    };
    if let Some(path) = set_aside {
        message = fill(
            // Translators: a toast's text goes on: {message} is what came
            // before ("Backup restored"), {path} a file's path.
            &gettext("{message}. The copy an earlier restore left waiting was kept as {path}"),
            &[("message", &message), ("path", &path.display().to_string())],
        );
    }
    message
}

/// The size of each monitor of `widget`'s display, in application pixels
/// (`gdk_monitor_get_geometry`), for fitting a saved window size to them.
fn monitor_sizes(widget: &impl IsA<gtk::Widget>) -> Vec<(i32, i32)> {
    let monitors = widget.display().monitors();
    (0..monitors.n_items())
        .filter_map(|i| monitors.item(i).and_downcast::<gtk::gdk::Monitor>())
        .map(|monitor| {
            let geometry = monitor.geometry();
            (geometry.width(), geometry.height())
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;

    /// A window opened as at startup on a data directory whose saved state
    /// reopens a deck of `cards` cards; not shown. Returns the window, its
    /// deck page and the directory, which must outlive them.
    fn reopened_with(cards: usize) -> (RappelWindow, DeckPage, tempfile::TempDir) {
        let (tmp, state) = crate::testing::temporary_state();
        let data_dir = state.data_dir().to_path_buf();
        let deck = state.create_deck("Deck").unwrap();
        for i in 0..cards {
            state
                .add_card_with_tags(&deck.id, &format!("front {i}"), "back", &[])
                .unwrap();
        }
        window_state::save(
            &data_dir,
            &window_state::WindowState {
                deck_id: Some(deck.id),
                ..window_state::WindowState::default()
            },
        );
        let window: RappelWindow = glib::Object::new();
        window.show_main(state);
        let page = window
            .imp()
            .navigation_view
            .visible_page()
            .and_downcast::<DeckPage>()
            .expect("the saved deck is reopened");
        (window, page, tmp)
    }

    /// The window's focus widget (it need not be shown for that).
    fn focus_of(window: &RappelWindow) -> gtk::Widget {
        gtk::prelude::RootExt::focus(window).expect("the window has a focus widget")
    }

    #[test]
    fn a_deck_reopened_at_startup_has_the_focus_on_its_first_card() {
        crate::testing::with_widgets(|| {
            let (window, page, _tmp) = reopened_with(3);
            let focus = focus_of(&window);
            assert!(focus.is_ancestor(&page), "not Back or the deck list");
            let list = focus
                .ancestor(gtk::ListView::static_type())
                .expect("a card of the list has the focus");
            assert!(list.is_ancestor(&page));
            window.destroy();
        });
    }

    /// Ctrl+F does not open All Cards over a deck page whose cards are
    /// being chosen, as it does not leave All Cards' selection.
    #[test]
    fn search_waits_while_a_deck_pages_cards_are_chosen() {
        crate::testing::with_widgets(|| {
            let (window, page, _tmp) = reopened_with(2);
            page.activate_action("selection.choose", None).unwrap();
            assert!(page.selecting());
            window.search();
            let visible = window.imp().navigation_view.visible_page();
            assert_eq!(visible, Some(page.clone().upcast()));
            page.activate_action("selection.cancel", None).unwrap();
            window.search();
            assert!(
                window
                    .imp()
                    .navigation_view
                    .visible_page()
                    .is_some_and(|p| p.is::<BrowserPage>())
            );
            window.destroy();
        });
    }

    /// The id of the deck row that has the focus, if one has.
    fn focused_deck(window: &RappelWindow) -> Option<String> {
        let focus = gtk::prelude::RootExt::focus(window)?;
        let row = if let Some(row) = focus.downcast_ref::<crate::widgets::DeckRow>() {
            row.clone()
        } else {
            focus
                .ancestor(crate::widgets::DeckRow::static_type())
                .and_downcast::<crate::widgets::DeckRow>()?
        };
        Some(row.deck_id())
    }

    /// Merge Into… from a deck page (ADR 0066): the page goes back to the
    /// deck list, focused on the deck it went into; the Undo brings the
    /// deck back, focused, with its cards.
    #[test]
    fn a_deck_merged_from_its_page_goes_back_to_the_list_and_its_undo_brings_it_back() {
        crate::testing::with_widgets(|| {
            let (window, page, _tmp) = reopened_with(2);
            let imp = window.imp();
            imp.navigation_view.set_animate_transitions(false);
            let state = imp.state.get().unwrap().clone();
            let source = page.deck_id().unwrap();
            let target = state.create_deck("Languages").unwrap();
            let source_deck = state.deck(&source).unwrap();
            let merged = state.merge_deck(&source, &target.id).unwrap();
            let done = crate::dialogs::merge_deck::Done {
                source: source_deck,
                target: target.clone(),
                merged: merged.clone(),
            };

            window.leave_merged_deck(&page, &state, done);

            let on_list = || {
                imp.navigation_view
                    .visible_page()
                    .is_some_and(|p| p.is::<DecksPage>())
            };
            assert!(crate::testing::settle_until(
                on_list,
                Duration::from_secs(5)
            ));
            let ids = || -> Vec<String> {
                imp.decks_page
                    .deck_rows()
                    .iter()
                    .map(crate::widgets::DeckRow::deck_id)
                    .collect()
            };
            assert_eq!(ids(), vec![target.id.clone()]);
            assert!(crate::testing::settle_until(
                || focused_deck(&window).as_deref() == Some(target.id.as_str()),
                Duration::from_secs(5)
            ));
            // The one deck left has nothing to merge into.
            assert!(!imp.decks_page.deck_rows()[0].can_merge());

            state.unmerge_deck(&merged).unwrap();
            window.deck_unmerged(&source);
            assert_eq!(ids(), [source.clone(), target.id.clone()]);
            assert_eq!(focused_deck(&window).as_deref(), Some(source.as_str()));
            assert!(
                imp.decks_page
                    .deck_rows()
                    .iter()
                    .all(crate::widgets::DeckRow::can_merge)
            );
            assert_eq!(state.card_count(&source).unwrap(), 2);
            window.destroy();
        });
    }

    /// A tag's session is no deck's (ADR 0069): closed during one started
    /// from All Cards, the window reopens on the deck list; started from a
    /// deck page, on that deck, as under a deck's session.
    #[test]
    fn a_tag_session_is_not_remembered_as_the_last_deck() {
        crate::testing::with_widgets(|| {
            let (window, page, _tmp) = reopened_with(1);
            let imp = window.imp();
            imp.navigation_view.set_animate_transitions(false);
            let state = imp.state.get().unwrap().clone();
            let deck = page.deck_id().unwrap();
            let card = &state.list_tagged_cards(&deck).unwrap()[0].card.id;
            state
                .add_tags_to_cards(
                    std::slice::from_ref(card),
                    &["verbs".to_owned()],
                    usize::MAX,
                )
                .unwrap();
            let tag_id = state.tags().unwrap()[0].id.clone();
            let saved = || {
                window.save_state();
                window_state::load(state.data_dir()).deck_id
            };
            // From the deck page: that deck is open under the session.
            page.activate_action("deck.study-tag", Some(&tag_id.to_variant()))
                .unwrap();
            assert!(
                imp.navigation_view
                    .visible_page()
                    .is_some_and(|p| p.is::<ReviewPage>())
            );
            assert_eq!(saved().as_deref(), Some(deck.as_str()));
            // From All Cards opened on the deck list: no deck.
            imp.navigation_view.pop_to_page(&*imp.decks_page);
            WidgetExt::activate_action(&window, "win.search", None).unwrap();
            let browser = imp
                .navigation_view
                .visible_page()
                .and_downcast::<BrowserPage>()
                .unwrap();
            browser
                .activate_action("browser.study-tag", Some(&tag_id.to_variant()))
                .unwrap();
            assert!(
                imp.navigation_view
                    .visible_page()
                    .is_some_and(|p| p.is::<ReviewPage>())
            );
            assert_eq!(saved(), None);
            window.destroy();
        });
    }

    /// A window closed on a deck that was then merged starts on the deck
    /// list: the merged deck is deleted, and reopening it does nothing.
    #[test]
    fn a_merged_last_deck_is_not_reopened() {
        crate::testing::with_widgets(|| {
            let (tmp, state) = crate::testing::temporary_state();
            let data_dir = state.data_dir().to_path_buf();
            let source = state.create_deck("Spanish").unwrap();
            let target = state.create_deck("Languages").unwrap();
            state.merge_deck(&source.id, &target.id).unwrap();
            window_state::save(
                &data_dir,
                &window_state::WindowState {
                    deck_id: Some(source.id),
                    ..window_state::WindowState::default()
                },
            );
            let window: RappelWindow = glib::Object::new();
            window.show_main(state);
            let visible = window.imp().navigation_view.visible_page();
            assert!(visible.is_some_and(|p| p.is::<DecksPage>()));
            window.destroy();
            drop(tmp);
        });
    }

    /// Archive from a deck page (ADR 0068): the page goes back to the deck
    /// list, focused on the row now at the deck's place, and the "Archived
    /// Decks" row shows; the Undo brings the deck back, focused.
    #[test]
    fn a_deck_archived_from_its_page_goes_back_to_the_list_and_its_undo_brings_it_back() {
        crate::testing::with_widgets(|| {
            let (window, page, _tmp) = reopened_with(2);
            let imp = window.imp();
            imp.navigation_view.set_animate_transitions(false);
            let state = imp.state.get().unwrap().clone();
            let source = page.deck_id().unwrap();
            let other = state.create_deck("Languages").unwrap();
            imp.decks_page.refresh();
            assert!(!imp.decks_page.archived_row_shown());

            page.activate_action("deck.archive", None).unwrap();

            let on_list = || {
                imp.navigation_view
                    .visible_page()
                    .is_some_and(|p| p.is::<DecksPage>())
            };
            assert!(crate::testing::settle_until(
                on_list,
                Duration::from_secs(5)
            ));
            let ids = || -> Vec<String> {
                imp.decks_page
                    .deck_rows()
                    .iter()
                    .map(crate::widgets::DeckRow::deck_id)
                    .collect()
            };
            assert_eq!(ids(), vec![other.id.clone()]);
            assert!(crate::testing::settle_until(
                || focused_deck(&window).as_deref() == Some(other.id.as_str()),
                Duration::from_secs(5)
            ));
            assert!(imp.decks_page.archived_row_shown());
            assert!(state.deck(&source).unwrap().is_archived());
            // The other deck has nothing to merge into: the archived one is
            // not offered.
            assert!(!imp.decks_page.deck_rows()[0].can_merge());

            // What the toast's Undo does.
            state.unarchive_deck(&source).unwrap();
            window.data_changed();
            imp.decks_page.deck_restored(&source);
            assert_eq!(ids(), [source.clone(), other.id.clone()]);
            assert_eq!(focused_deck(&window).as_deref(), Some(source.as_str()));
            assert!(!imp.decks_page.archived_row_shown());
            assert_eq!(state.card_count(&source).unwrap(), 2);
            window.destroy();
        });
    }

    /// The Undo of "Deck unarchived" archives as Archive does (ADR 0068):
    /// the page of the deck, opened since, is left for the deck list, which
    /// focuses the row now at its place; on the Archived Decks page the
    /// deck comes back with the focus on its Unarchive button.
    #[test]
    fn undoing_an_unarchive_leaves_the_decks_page_and_keeps_the_focus() {
        use crate::pages::ArchivedPage;
        crate::testing::with_widgets(|| {
            let (window, page, _tmp) = reopened_with(2);
            let imp = window.imp();
            imp.navigation_view.set_animate_transitions(false);
            let state = imp.state.get().unwrap().clone();
            let id = page.deck_id().unwrap();
            let other = state.create_deck("Languages").unwrap();
            let zoo = state.create_deck("Zoo").unwrap();
            state.archive_deck(&zoo.id).unwrap();
            imp.decks_page.refresh();

            // The deck's page is open: it is left for the deck list.
            window.rearchive_deck(&state, &id).unwrap();
            let on_list = || {
                imp.navigation_view
                    .visible_page()
                    .is_some_and(|p| p.is::<DecksPage>())
            };
            assert!(crate::testing::settle_until(
                on_list,
                Duration::from_secs(5)
            ));
            assert!(state.deck(&id).unwrap().is_archived());
            assert!(crate::testing::settle_until(
                || focused_deck(&window).as_deref() == Some(other.id.as_str()),
                Duration::from_secs(5)
            ));
            // A second Undo finds it archived already: nothing changes.
            assert!(matches!(
                window.rearchive_deck(&state, &id),
                Err(rappel_core::Error::DeckArchived(_))
            ));

            // From the Archived Decks page: Unarchive, then its Undo.
            gtk::prelude::WidgetExt::activate(&*imp.decks_page.imp().archived_row);
            let archived_page = || {
                imp.navigation_view
                    .visible_page()
                    .and_downcast::<ArchivedPage>()
            };
            assert!(crate::testing::settle_until(
                || archived_page().is_some(),
                Duration::from_secs(5)
            ));
            let archived = archived_page().unwrap();
            archived.unarchive(&id);
            assert_eq!(archived.deck_ids(), std::slice::from_ref(&zoo.id));
            window.rearchive_deck(&state, &id).unwrap();
            assert_eq!(archived.deck_ids(), [id.clone(), zoo.id.clone()]);
            let button = archived.unarchive_button(&id).unwrap();
            assert_eq!(
                gtk::prelude::RootExt::focus(&window),
                Some(button.upcast::<gtk::Widget>())
            );
            window.destroy();
        });
    }

    /// Hardening round 4 (ADR 0073): archive and unarchive racing an open
    /// deck page. The deck's page has a study session open over it when
    /// the Undo of an older "Deck unarchived" archives the deck: both pages
    /// leave for the deck list, leaving writes no log entry, and a
    /// second Undo (the toast pressed again) changes nothing. Then the deck
    /// is unarchived and archived again from its page in a row, as fast as
    /// the code allows: one stamp, the list right each time.
    #[test]
    fn archiving_races_with_an_open_deck_page_and_its_session() {
        crate::testing::with_widgets(|| {
            let (window, page, _tmp) = reopened_with(3);
            let imp = window.imp();
            imp.navigation_view.set_animate_transitions(false);
            let state = imp.state.get().unwrap().clone();
            let id = page.deck_id().unwrap();
            let other = state.create_deck("Other").unwrap();
            page.activate_action("deck.study", None).unwrap();
            let on_review = || {
                imp.navigation_view
                    .visible_page()
                    .is_some_and(|p| p.is::<ReviewPage>())
            };
            assert!(crate::testing::settle_until(
                on_review,
                Duration::from_secs(30)
            ));
            let log = |state: &AppState| -> usize {
                state
                    .list_tagged_cards(&id)
                    .unwrap()
                    .iter()
                    .map(|c| state.review_events(&c.card.id).unwrap().len())
                    .sum()
            };
            let before = log(&state);

            window.rearchive_deck(&state, &id).unwrap();
            let on_list = || {
                imp.navigation_view
                    .visible_page()
                    .is_some_and(|p| p.is::<DecksPage>())
            };
            assert!(crate::testing::settle_until(
                on_list,
                Duration::from_secs(30)
            ));
            assert!(!window.pages().iter().any(ObjectExt::is::<ReviewPage>));
            assert!(!window.pages().iter().any(ObjectExt::is::<DeckPage>));
            let stamp = state.deck(&id).unwrap().archived_at;
            assert!(stamp.is_some());
            assert!(matches!(
                window.rearchive_deck(&state, &id),
                Err(rappel_core::Error::DeckArchived(_))
            ));
            assert_eq!(state.deck(&id).unwrap().archived_at, stamp);
            // Leaving the session on the archive wrote no log entry (no answer
            // was given in it, so this checks that leaving itself writes
            // nothing, not that a later answer is refused).
            assert_eq!(log(&state), before, "leaving the session wrote nothing");
            let listed = || -> Vec<String> {
                imp.decks_page
                    .deck_rows()
                    .iter()
                    .map(crate::widgets::DeckRow::deck_id)
                    .collect()
            };
            assert_eq!(listed(), std::slice::from_ref(&other.id));

            // Unarchive, open the page again, archive from it at once.
            state.unarchive_deck(&id).unwrap();
            window.data_changed();
            assert_eq!(listed(), [id.clone(), other.id.clone()]);
            imp.decks_page.reopen(&id);
            let page = imp
                .navigation_view
                .visible_page()
                .and_downcast::<DeckPage>()
                .expect("the deck's page is open again");
            page.activate_action("deck.archive", None).unwrap();
            assert!(crate::testing::settle_until(
                on_list,
                Duration::from_secs(30)
            ));
            assert!(state.deck(&id).unwrap().is_archived());
            assert_eq!(listed(), std::slice::from_ref(&other.id));
            assert!(imp.decks_page.archived_row_shown());
            window.destroy();
        });
    }

    /// A window closed on a deck that was then archived starts on the
    /// deck list, as for a deleted deck (ADR 0068).
    #[test]
    fn an_archived_last_deck_is_not_reopened() {
        crate::testing::with_widgets(|| {
            let (tmp, state) = crate::testing::temporary_state();
            let data_dir = state.data_dir().to_path_buf();
            let deck = state.create_deck("Spanish").unwrap();
            state.archive_deck(&deck.id).unwrap();
            window_state::save(
                &data_dir,
                &window_state::WindowState {
                    deck_id: Some(deck.id),
                    ..window_state::WindowState::default()
                },
            );
            let window: RappelWindow = glib::Object::new();
            window.show_main(state);
            let visible = window.imp().navigation_view.visible_page();
            assert!(visible.is_some_and(|p| p.is::<DecksPage>()));
            window.destroy();
            drop(tmp);
        });
    }

    /// Every deck archived: the empty state says so, with Create Deck and
    /// the "Archived Decks" row; that page lists the deck with the focus on
    /// its Unarchive button, and the last Unarchive goes back to the deck
    /// list, focused on the deck.
    #[test]
    fn with_every_deck_archived_the_list_leads_to_them_and_back() {
        use crate::pages::ArchivedPage;
        crate::testing::with_widgets(|| {
            let (tmp, state) = crate::testing::temporary_state();
            let deck = state.create_deck("Spanish").unwrap();
            state
                .add_card_with_tags(&deck.id, "hola", "hello", &[])
                .unwrap();
            state.archive_deck(&deck.id).unwrap();
            let window: RappelWindow = glib::Object::new();
            window.show_main(state.clone());
            let imp = window.imp();
            imp.navigation_view.set_animate_transitions(false);
            let decks = imp.decks_page.imp();
            assert_eq!(
                decks.content_stack.visible_child_name().as_deref(),
                Some("empty")
            );
            assert_eq!(decks.empty_status.title(), "Every Deck Is Archived");
            assert!(imp.decks_page.archived_row_shown());
            assert_eq!(decks.archived_empty_count.label(), "1");
            assert!(decks.empty_new_button.get_visible() && decks.empty_new_button.is_sensitive());

            gtk::prelude::WidgetExt::activate(&*decks.archived_empty_row);
            let archived_page = || {
                imp.navigation_view
                    .visible_page()
                    .and_downcast::<ArchivedPage>()
            };
            assert!(crate::testing::settle_until(
                || archived_page().is_some(),
                Duration::from_secs(5)
            ));
            let page = archived_page().unwrap();
            assert_eq!(page.deck_ids(), std::slice::from_ref(&deck.id));
            assert!(crate::testing::settle_until(
                || {
                    gtk::prelude::RootExt::focus(&window)
                        .is_some_and(|f| f.is::<gtk::Button>() && f.is_ancestor(&page))
                },
                Duration::from_secs(5)
            ));

            page.unarchive(&deck.id);
            assert!(crate::testing::settle_until(
                || focused_deck(&window).as_deref() == Some(deck.id.as_str()),
                Duration::from_secs(5)
            ));
            assert!(!imp.decks_page.archived_row_shown());
            assert!(!state.deck(&deck.id).unwrap().is_archived());
            window.destroy();
            drop(tmp);
        });
    }

    #[test]
    fn an_empty_deck_reopened_at_startup_has_the_focus_on_add_card() {
        crate::testing::with_widgets(|| {
            let (window, page, _tmp) = reopened_with(0);
            let focus = focus_of(&window);
            assert!(focus.is_ancestor(&page));
            let button = focus.downcast::<gtk::Button>().expect("a button");
            assert_eq!(button.action_name().as_deref(), Some("deck.new-card"));
            window.destroy();
        });
    }

    #[test]
    fn recovery_toasts_say_what_happened_and_where_the_old_file_is() {
        let kept = Path::new("/data/rappel.sqlite3.replaced-20261015-120000");
        assert_eq!(
            recovered_message(Recovered::Backup, Some(kept), None),
            "Backup restored. The previous database was kept as \
             /data/rappel.sqlite3.replaced-20261015-120000"
        );
        assert_eq!(
            recovered_message(Recovered::Backup, None, None),
            "Backup restored"
        );
        assert_eq!(
            recovered_message(Recovered::Finished, None, None),
            "Restore finished"
        );
        assert_eq!(
            recovered_message(Recovered::Fresh, Some(kept), None),
            "Started fresh. The previous database was kept as \
             /data/rappel.sqlite3.replaced-20261015-120000"
        );
        assert_eq!(
            recovered_message(Recovered::Fresh, None, None),
            "Started with an empty database"
        );
        let copy = Path::new("/data/rappel.sqlite3.restore.refused-20261015-120000");
        assert_eq!(
            recovered_message(Recovered::Fresh, Some(kept), Some(copy)),
            "Started fresh. The previous database was kept as \
             /data/rappel.sqlite3.replaced-20261015-120000. The copy an earlier restore left \
             waiting was kept as /data/rappel.sqlite3.restore.refused-20261015-120000"
        );
        let aside = Path::new("/data/rappel.sqlite3.restore.set-aside-20261015-120000");
        assert_eq!(
            recovered_message(Recovered::Backup, Some(kept), Some(aside)),
            "Backup restored. The previous database was kept as \
             /data/rappel.sqlite3.replaced-20261015-120000. The copy an earlier restore left \
             waiting was kept as /data/rappel.sqlite3.restore.set-aside-20261015-120000"
        );
        // Each fits a toast whole, or starts with a title that does, in
        // English and in French.
        let check = || {
            for how in [Recovered::Backup, Recovered::Finished, Recovered::Fresh] {
                for replaced in [None, Some(kept)] {
                    for set_aside in [None, Some(aside)] {
                        let text = recovered_message(how, replaced, set_aside);
                        if let crate::ui::toast::ToastText::Detailed { title, .. } =
                            crate::ui::toast::toast_text(&text)
                        {
                            assert_ne!(title, crate::ui::toast::fallback_title(), "{text}");
                        }
                    }
                }
            }
        };
        check();
        crate::i18n::tests::in_french(check);
    }

    /// A damaged stored setting at startup shows "A Setting Is Damaged" with
    /// Reset This Setting focused; the reset shows the main page (ADR 0052).
    #[test]
    fn a_damaged_setting_at_startup_can_be_reset_from_its_page() {
        crate::testing::with_widgets(|| {
            let tmp = tempfile::tempdir().expect("a temporary directory");
            let data = crate::pages::damaged_setting_folder(
                &tmp,
                &[rappel_core::domain::SettingKey::DesiredRetention],
            );
            let err = AppState::open(data.clone()).expect_err("the open is refused");
            let problem = crate::startup::open_problem(&err, &data);
            let window: RappelWindow = glib::Object::new();
            window.show_problem(problem);
            let imp = window.imp();
            assert_eq!(imp.stack.visible_child_name().as_deref(), Some("setting"));
            let focus = focus_of(&window);
            let button = focus
                .downcast::<gtk::Button>()
                .expect("a button has the focus");
            assert_eq!(button.label().as_deref(), Some("_Reset This Setting"));

            glib::MainContext::default().block_on(imp.setting_page.reset());

            assert_eq!(imp.stack.visible_child_name().as_deref(), Some("main"));
            let settings = imp.state.get().expect("a database is open").settings();
            assert!(settings.is_ok());
            window.destroy();
        });
    }

    /// Finish Restore on a waiting copy from a newer Rappel shows the "too
    /// new" page, as a database that is too new does, with its version
    /// (ADR 0050, row 35).
    #[test]
    fn a_waiting_copy_from_a_newer_rappel_shows_the_too_new_page() {
        crate::testing::with_widgets(|| {
            let tmp = tempfile::tempdir().expect("a temporary directory");
            let (data, partial) = crate::pages::interrupted_restore_folder(&tmp, false);
            // `user_version`, bytes 60 to 63 of the header, big-endian.
            let mut bytes = std::fs::read(&partial).expect("the copy is read");
            bytes[60..64].copy_from_slice(&10_000_u32.to_be_bytes());
            std::fs::write(&partial, bytes).expect("the copy is written");
            let window: RappelWindow = glib::Object::new();
            window.show_problem(Problem::Recovery {
                reason: crate::startup::RecoveryReason::InterruptedRestore,
                data_dir: data,
            });
            let imp = window.imp();
            assert_eq!(imp.stack.visible_child_name().as_deref(), Some("recovery"));
            glib::MainContext::default().block_on(imp.recovery_page.finish_restore());
            assert_eq!(imp.stack.visible_child_name().as_deref(), Some("too-new"));
            let description = imp.too_new_page.description().unwrap_or_default();
            assert!(
                description.contains("database version 10000"),
                "{description}"
            );
            assert!(partial.exists(), "the copy is kept");
            window.destroy();
        });
    }
}
