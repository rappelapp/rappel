//! The root page: the list of decks, or an empty state, with create,
//! rename, merge, archive and delete (the last three undone from a toast).
//! Activating a deck opens its page. While a deck is archived, an
//! "Archived Decks" row at the end (or under the empty state) opens the
//! Archived Decks page (ADR 0068). Layout in `data/decks_page.blp`.
//!
//! Each row says what its deck has to study, or when it next will
//! ("Next review in 3 h", `widgets::deck_row::text`). Those texts age, so
//! the list is read again in place (the rows and the focus stay) when the
//! page is shown, when the window becomes active (after a suspend the
//! timer below is late: `GLib` timeouts use the monotonic clock), and by a
//! timer at the next change, at most a minute, ten minutes or an hour
//! away (`refresh_delay_ms`, ADR 0041).

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::time::{Duration, Instant};

use adw::prelude::*;
use adw::subclass::prelude::*;
use gtk::glib;
use rappel_core::domain::Deck;
use rappel_core::scheduling::StudyScope;

use crate::dialogs::deck_name_dialog::{self, Purpose};
use crate::dialogs::merge_deck;
use crate::i18n::gettext;
use crate::pages::{
    ArchivedPage, DeckPage, ReviewPage, StartedFrom, StatsPage, Studied, archived_text,
};
use crate::state::AppState;
use crate::ui::focus::index_after_removal;
use crate::ui::text::with_error;
use crate::ui::toast::{Message, show_error, show_error_message, show_toast_from, undo_toast};
use crate::ui::{keys, nav};
use crate::widgets::DeckRow;
use crate::widgets::deck_row::text::refresh_delay_ms;
use crate::window::RappelWindow;

/// The empty state's icon, as in its layout, given back after a short
/// window took it off (`ui::breakpoint::compact_status_page`).
const EMPTY_ICON: &str = "accessories-dictionary-symbolic";

mod imp {
    use std::cell::{Cell, OnceCell, RefCell};

    use adw::prelude::*;
    use adw::subclass::prelude::*;
    use gtk::glib;

    use crate::state::AppState;
    use crate::ui::debounce::Debounce;
    use crate::ui::rows;
    use crate::ui::toast::ErrorSlot;

    #[derive(Debug, Default, gtk::CompositeTemplate)]
    #[template(resource = "/io/github/rappelapp/Rappel/decks_page.ui")]
    pub struct DecksPage {
        #[template_child]
        pub empty_short: TemplateChild<adw::Breakpoint>,
        #[template_child]
        pub empty_status: TemplateChild<adw::StatusPage>,
        #[template_child]
        pub content_stack: TemplateChild<gtk::Stack>,
        #[template_child]
        pub deck_list: TemplateChild<gtk::ListBox>,
        #[template_child]
        pub empty_new_button: TemplateChild<gtk::Button>,
        /// S pressed and not yet released (`add_study_key`).
        pub start_hold: crate::ui::keys::StartHold<String>,
        #[template_child]
        pub archived_list: TemplateChild<gtk::ListBox>,
        #[template_child]
        pub archived_row: TemplateChild<adw::ActionRow>,
        #[template_child]
        pub archived_count: TemplateChild<gtk::Label>,
        #[template_child]
        pub archived_empty_clamp: TemplateChild<adw::Clamp>,
        #[template_child]
        pub archived_empty_list: TemplateChild<gtk::ListBox>,
        #[template_child]
        pub archived_empty_row: TemplateChild<adw::ActionRow>,
        #[template_child]
        pub archived_empty_count: TemplateChild<gtk::Label>,
        /// Set once by the window when the database is open (ADR 0011).
        pub state: OnceCell<AppState>,
        /// A name dialog is open; a second one (double click, Ctrl+N) is
        /// not opened on top of it.
        pub asking: Cell<bool>,
        /// The deck whose page was opened last, focused when coming back.
        pub opened: RefCell<Option<String>>,
        /// Coming back from a deck page whose deck left the list (it was
        /// archived): the row now at this place is focused instead.
        pub return_index: Cell<Option<usize>>,
        /// Reads the decks again when a row's text would change.
        pub refresh_timer: Debounce,
        /// The last "could not be read" toast of a reload.
        pub read_errors: ErrorSlot,
        /// The window's `is-active` is watched (connected once, at the
        /// first realize).
        pub watching_window: Cell<bool>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for DecksPage {
        const NAME: &'static str = "RappelDecksPage";
        type Type = super::DecksPage;
        type ParentType = adw::NavigationPage;

        fn class_init(klass: &mut Self::Class) {
            klass.bind_template();
            // Activated by the deck rows' menus with the deck id.
            rows::install_id_action(klass, "decks.rename", |page, id| {
                page.rename(id.to_owned());
            });
            rows::install_id_action(klass, "decks.practice", super::DecksPage::practise);
            rows::install_id_action(klass, "decks.merge", super::DecksPage::merge);
            rows::install_id_action(klass, "decks.archive", super::DecksPage::archive);
            rows::install_id_action(klass, "decks.delete", super::DecksPage::delete);
            klass.install_action("decks.statistics", None, |page, _, _| {
                page.show_statistics();
            });
        }

        fn instance_init(obj: &glib::subclass::InitializingObject<Self>) {
            obj.init_template();
        }
    }

    impl ObjectImpl for DecksPage {
        fn constructed(&self) {
            self.parent_constructed();
            // The empty state on a short window (ADR 0058).
            crate::ui::breakpoint::compact_status_page(
                &self.empty_short,
                &self.empty_status,
                super::EMPTY_ICON,
            );
            let page = self.obj();
            self.deck_list.connect_row_activated(glib::clone!(
                #[weak]
                page,
                move |_, row| {
                    if let Some(row) = row.downcast_ref::<crate::widgets::DeckRow>() {
                        page.open(&row.deck_id());
                    }
                }
            ));
            // Back from a deck: its counts may have changed, and every
            // row's "Next review in …" may be out of date. The focus is set
            // here, not before the pop: AdwNavigationView focuses the page
            // when its transition ends, which is when `shown` is emitted
            // (CLAUDE.md, "Focus").
            page.connect_shown(|page| {
                let imp = page.imp();
                let opened = imp.opened.take();
                match (imp.return_index.take(), opened) {
                    (Some(index), _) => page.reload_or_toast(super::Focus::Index(index)),
                    (None, Some(id)) => page.reload_or_toast(super::Focus::Deck(&id)),
                    (None, None) => page.reload_or_toast(super::Focus::Keep),
                }
            });
            // Both "Archived Decks" rows (under the list, and under the
            // empty state) open the Archived Decks page.
            for list in [&self.archived_list, &self.archived_empty_list] {
                list.connect_row_activated(glib::clone!(
                    #[weak]
                    page,
                    move |_, _| page.show_archived()
                ));
            }
            page.connect_realize(super::DecksPage::watch_window);
            page.add_study_key();
        }
    }
    impl WidgetImpl for DecksPage {}
    impl NavigationPageImpl for DecksPage {}

    impl DecksPage {
        pub fn state(&self) -> Option<&AppState> {
            self.state.get()
        }
    }
}

glib::wrapper! {
    pub struct DecksPage(ObjectSubclass<imp::DecksPage>)
        @extends adw::NavigationPage, gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}

/// Which row gets the keyboard focus after the list is rebuilt.
#[derive(Debug, Clone, Copy)]
enum Focus<'a> {
    Keep,
    Deck(&'a str),
    /// The row at this position, or the last one if the list got shorter.
    Index(usize),
}

impl DecksPage {
    /// Gives the page the open database and shows its decks.
    ///
    /// # Errors
    /// If the decks cannot be read.
    pub fn set_state(&self, state: AppState) -> rappel_core::Result<()> {
        if self.imp().state.set(state).is_err() {
            tracing::error!("the deck page already has an open database");
        }
        self.reload(Focus::Keep)
    }

    /// Reads the decks again, e.g. after a card change undone from
    /// another page.
    pub fn refresh(&self) {
        self.reload_or_toast(Focus::Keep);
    }

    /// Reads the decks again when the window becomes active, while this
    /// page shows: after a suspend or a long time in the background, the
    /// timer has not yet caught up (see the module docs).
    fn watch_window(&self) {
        let imp = self.imp();
        if imp.watching_window.get() {
            return;
        }
        let Some(window) = self.root().and_downcast::<gtk::Window>() else {
            return;
        };
        imp.watching_window.set(true);
        window.connect_is_active_notify(glib::clone!(
            #[weak(rename_to = page)]
            self,
            move |window| {
                if window.is_active() && page.is_on_top() {
                    page.refresh();
                }
            }
        ));
    }

    /// Whether this page is the one the navigation view shows.
    fn is_on_top(&self) -> bool {
        nav::navigation_view(self)
            .and_then(|view| view.visible_page())
            .is_some_and(|visible| visible == *self.upcast_ref::<adw::NavigationPage>())
    }

    /// Reads the decks again after `delay_ms`, replacing a pending read.
    /// Under another page it does nothing: `shown` reads them on the way
    /// back. Holds the page weakly: a closed window ends it.
    fn arm_refresh(&self, delay_ms: u64) {
        let page = self.downgrade();
        self.imp()
            .refresh_timer
            .restart(Duration::from_millis(delay_ms), move || {
                if let Some(page) = page.upgrade()
                    && page.is_on_top()
                {
                    page.refresh();
                }
            });
    }

    /// At startup: pushes the page of the deck that was open when the
    /// window last closed. A deck deleted or archived since leaves the deck
    /// list showing (`open` ignores `DeckNotFound` and archived decks).
    pub fn reopen(&self, id: &str) {
        self.open(id);
    }

    /// Pushes the page of deck `id`.
    fn open(&self, id: &str) {
        let Some(state) = self.imp().state() else {
            return;
        };
        let deck = match state.deck(id) {
            // An archived deck has no page (ADR 0068): the deck list stays.
            Ok(deck) if deck.is_archived() => return,
            Ok(deck) => deck,
            // Deleted meanwhile, e.g. from another window's toast: the
            // list is rebuilt by that change.
            Err(rappel_core::Error::DeckNotFound(_)) => return,
            Err(err) => {
                // Translators: a toast: what is before ":" is its title, the rest is behind Details. {error} is why, in English.
                self.error_toast(&with_error(
                    &gettext("The deck could not be read: {error}"),
                    &err,
                ));
                return;
            }
        };
        let Some(view) = nav::navigation_view(self) else {
            return;
        };
        // A mouse double click activates a row once: GtkListBox activates
        // only on n_press == 1. Key repeat is what the helper guards.
        let started = Instant::now();
        let mut built_ms = 0;
        let pushed = nav::push_unless_on_top(&view, || {
            let page = DeckPage::new(state.clone(), deck)?;
            built_ms = started.elapsed().as_millis();
            self.imp().opened.replace(Some(id.to_owned()));
            Ok::<_, rappel_core::Error>(page)
        });
        match pushed {
            Ok(true) => log_first_frame(&view, started, built_ms),
            Ok(false) => {}
            // Translators: a toast: what is before ":" is its title, the rest is behind Details. {error} is why, in English.
            Err(err) => self.error_toast(&with_error(
                &gettext("The cards could not be read: {error}"),
                &err,
            )),
        }
    }

    /// S on a focused deck row studies that deck (ADR 0081), as the Study
    /// button on its page does, without opening the page. S acts on its
    /// *release* (`keys::StartHold`: a held S repeats, and the session's own
    /// S would suspend cards). Bubble phase (the default of a
    /// `GtkEventControllerKey`, gtkeventcontrollerkey.c): a focused text
    /// field has had the key first. The focus is on "a row" when it is the
    /// row or its menu button (`gtk_widget_get_ancestor` counts the widget
    /// itself); the Archived Decks row and the empty state's button are not
    /// deck rows. A popover or a dialog cannot leak its S here: a popover's
    /// native surface stops key events at its own root, and an `AdwDialog`
    /// is not a descendant of the page.
    fn add_study_key(&self) {
        let keys = gtk::EventControllerKey::new();
        keys.connect_key_pressed(glib::clone!(
            #[weak(rename_to = page)]
            self,
            #[upgrade_or]
            glib::Propagation::Proceed,
            move |_, key, _, modifiers| page.study_key_pressed(key, modifiers)
        ));
        keys.connect_key_released(glib::clone!(
            #[weak(rename_to = page)]
            self,
            move |_, key, _, modifiers| page.study_key_released(key, modifiers)
        ));
        self.add_controller(keys);
        // Every press, before any child can claim it (Return on a row or
        // on a menu button never reaches the bubble phase above): anything
        // but a plain S cancels a held S. Returns Proceed, so it claims
        // neither the press nor its release (gtkeventcontrollerkey.c).
        let observer = gtk::EventControllerKey::new();
        observer.set_propagation_phase(gtk::PropagationPhase::Capture);
        observer.connect_key_pressed(glib::clone!(
            #[weak(rename_to = page)]
            self,
            #[upgrade_or]
            glib::Propagation::Proceed,
            move |_, key, _, modifiers| {
                page.imp().start_hold.observe_press(key, modifiers);
                glib::Propagation::Proceed
            }
        ));
        self.add_controller(observer);
        // A hold left behind would start a session at a later release.
        keys::when_left(self, |page| {
            if let Some(page) = page.downcast_ref::<Self>() {
                page.imp().start_hold.clear();
            }
        });
    }

    /// The deck row that has the keyboard focus, or holds the widget that
    /// has it.
    fn focused_row(&self) -> Option<DeckRow> {
        self.root()
            .and_then(|root| gtk::prelude::RootExt::focus(&root))
            .and_then(|focus| focus.ancestor(DeckRow::static_type()))
            .and_downcast::<DeckRow>()
    }

    /// Every key press or repeat the page sees: S on a deck row is
    /// swallowed (its release starts the session), and the row's deck is
    /// remembered; any other key cancels a held S.
    fn study_key_pressed(
        &self,
        key: gtk::gdk::Key,
        modifiers: gtk::gdk::ModifierType,
    ) -> glib::Propagation {
        let typing = keys::text_has_focus(self);
        let target = self.focused_row().map(|row| row.deck_id());
        if self
            .imp()
            .start_hold
            .pressed(key, modifiers, typing, target)
        {
            glib::Propagation::Stop
        } else {
            glib::Propagation::Proceed
        }
    }

    /// A key release: a plain S whose press was the last one seen here,
    /// on the same deck's row as now, studies that deck.
    fn study_key_released(&self, key: gtk::gdk::Key, modifiers: gtk::gdk::ModifierType) {
        let typing = keys::text_has_focus(self);
        let target = self.focused_row().map(|row| row.deck_id());
        if self
            .imp()
            .start_hold
            .released(key, modifiers, typing, target.as_ref())
            && let Some(id) = target
        {
            self.study(&id);
        }
    }

    /// Starts a session on deck `id` when it has something to study, else
    /// does nothing (the Study button is disabled then).
    fn study(&self, id: &str) {
        let Some(state) = self.imp().state().cloned() else {
            return;
        };
        let Some(view) = nav::navigation_view(self) else {
            return;
        };
        let pushed = nav::push_unless_on_top(&view, || {
            let deck = match state.deck(id) {
                // As `open`: deleted meanwhile, or archived (no page).
                Err(rappel_core::Error::DeckNotFound(_)) => return Err(None),
                Ok(deck) if deck.is_archived() => return Err(None),
                Ok(deck) => deck,
                Err(err) => return Err(Some(err)),
            };
            let scope = StudyScope::deck(&deck.id);
            if state
                .study_input(&scope, rappel_core::time::now_ms())
                .map_err(Some)?
                .is_empty()
            {
                return Err(None);
            }
            let studied = Studied::Deck {
                deck,
                from: StartedFrom::DeckList,
            };
            // Nothing to refresh on a save: the list reads its counts again
            // when it shows.
            ReviewPage::study(state.clone(), studied, |_| {}).map_err(Some)
        });
        // `Err(None)`: nothing to study.
        if let Err(Some(err)) = pushed {
            // Translators: a toast: what is before ":" is its title, the rest is behind Details. {error} is why, in English.
            self.error_toast(&with_error(
                &gettext("The session could not start: {error}"),
                &err,
            ));
        }
    }

    /// Pushes the statistics of every deck.
    fn show_statistics(&self) {
        let Some(state) = self.imp().state() else {
            return;
        };
        let Some(view) = nav::navigation_view(self) else {
            return;
        };
        if let Err(err) = nav::push_unless_on_top(&view, || StatsPage::new(state, None)) {
            // Translators: a toast: what is before ":" is its title, the rest is behind Details. {error} is why, in English.
            self.error_toast(&with_error(
                &gettext("The statistics could not be read: {error}"),
                &err,
            ));
        }
    }

    /// Pushes the Archived Decks page.
    fn show_archived(&self) {
        let Some(state) = self.imp().state().cloned() else {
            return;
        };
        let Some(view) = nav::navigation_view(self) else {
            return;
        };
        if let Err(err) = nav::push_unless_on_top(&view, || ArchivedPage::new(state)) {
            // Translators: a toast: what is before ":" is its title, the rest is behind Details. {error} is why, in English.
            self.error_toast(&with_error(
                &gettext("The decks could not be read: {error}"),
                &err,
            ));
        }
    }

    /// Practice from deck `id`'s menu: pushes a practice of the deck, which
    /// writes nothing (ADR 0070); its Back and its summary's Back to Decks
    /// come back here.
    fn practise(&self, id: &str) {
        let Some(state) = self.imp().state().cloned() else {
            return;
        };
        let Some(view) = nav::navigation_view(self) else {
            return;
        };
        let pushed = nav::push_unless_on_top(&view, || {
            let studied = Studied::Deck {
                deck: state.deck(id)?,
                from: StartedFrom::DeckList,
            };
            // Nothing to refresh on a save: the list reads its counts again
            // when it shows.
            ReviewPage::practice(state.clone(), studied, |_| {})
        });
        if let Err(err) = pushed {
            // Translators: a toast: what is before ":" is its title, the rest is behind Details. {error} is why, in English.
            self.error_toast(&with_error(
                &gettext("The session could not start: {error}"),
                &err,
            ));
        }
    }

    /// Archives deck `id` at once: its row leaves the list (the focus on
    /// the row now at its place, `index_after_removal`), and the toast's
    /// Undo brings it back (ADR 0068).
    fn archive(&self, id: &str) {
        let Some(state) = self.imp().state().cloned() else {
            return;
        };
        let Some(index) = self.rows().iter().position(|row| row.deck_id() == id) else {
            return;
        };
        let deck = match state.archive_deck(id) {
            Ok(deck) => deck,
            Err(err) => {
                self.error_toast(&archived_text::not_archived(&err));
                return;
            }
        };
        if let Some(window) = self.root().and_downcast::<crate::window::RappelWindow>() {
            window.deck_archived(&deck.id);
        }
        self.reload_or_toast(Focus::Index(index));
        self.show_archived_toast(&state, deck.id);
    }

    /// Shows "Deck archived" with Undo, which unarchives deck `id` and has
    /// every page read its decks again, with the focus on the deck if the
    /// deck list shows. From this page, the window's root, which lives as
    /// long as it: a deck page that archived its deck is gone by then.
    pub fn show_archived_toast(&self, state: &AppState, id: String) {
        let state = state.clone();
        let page = self.downgrade();
        let toast = undo_toast(&archived_text::archived_toast(), move || {
            let Some(page) = page.upgrade() else {
                return;
            };
            match state.unarchive_deck(&id) {
                Ok(_) => {
                    if let Some(window) = page.root().and_downcast::<RappelWindow>() {
                        window.data_changed();
                    }
                    page.deck_restored(&id);
                }
                Err(err) => page.error_toast(&archived_text::not_unarchived(&err)),
            }
        });
        show_toast_from(self, toast);
    }

    /// After a deck left the list from elsewhere (an Undo): if this page
    /// shows, the row now at its place takes the focus at once
    /// ([`Self::focus_place_on_return`] noted it); else on the next
    /// `shown`.
    pub fn deck_left_list(&self) {
        if self.is_on_top()
            && let Some(index) = self.imp().return_index.take()
        {
            self.reload_or_toast(Focus::Index(index));
        }
    }

    /// Makes the row now at deck `id`'s place take the focus the next time
    /// this page shows: deck `id`, whose page is open, is leaving the list
    /// (ADR 0068).
    pub fn focus_place_on_return(&self, id: &str) {
        if let Some(index) = self.rows().iter().position(|row| row.deck_id() == id) {
            self.imp().return_index.set(Some(index));
        }
    }

    /// Asks for a name and creates the deck.
    pub fn new_deck(&self) {
        let Some(state) = self.imp().state().cloned() else {
            return;
        };
        glib::spawn_future_local(glib::clone!(
            #[weak(rename_to = page)]
            self,
            async move {
                let Some(decks) = page.named_decks(&state) else {
                    return;
                };
                let Some(name) = page.ask(Purpose::Create, decks).await else {
                    return;
                };
                match state.create_deck(&name) {
                    Ok(deck) => page.reload_or_toast(Focus::Deck(&deck.id)),
                    // Translators: a toast: what is before ":" is its title, the rest is behind Details. {error} is why, in English.
                    Err(err) => page.error_toast(&with_error(
                        &gettext("The deck could not be created: {error}"),
                        &err,
                    )),
                }
            }
        ));
    }

    fn rename(&self, id: String) {
        let Some(state) = self.imp().state().cloned() else {
            return;
        };
        glib::spawn_future_local(glib::clone!(
            #[weak(rename_to = page)]
            self,
            async move {
                let Some(decks) = page.named_decks(&state) else {
                    return;
                };
                let Some(deck) = decks.iter().find(|d| d.id == id).cloned() else {
                    return;
                };
                let purpose = Purpose::Rename(deck);
                let Some(name) = page.ask(purpose, decks).await else {
                    return;
                };
                match state.rename_deck(&id, &name) {
                    Ok(()) => page.reload_or_toast(Focus::Deck(&id)),
                    // Translators: a toast: what is before ":" is its title, the rest is behind Details. {error} is why, in English.
                    Err(err) => page.error_toast(&with_error(
                        &gettext("The deck could not be renamed: {error}"),
                        &err,
                    )),
                }
            }
        ));
    }

    /// Deletes at once; the toast's Undo brings the deck back.
    fn delete(&self, id: &str) {
        let Some(state) = self.imp().state() else {
            return;
        };
        let Some(decks) = self.decks(state) else {
            return;
        };
        let Some(index) = decks.iter().position(|d| d.id == id) else {
            return;
        };
        let name = decks[index].name.clone();
        if let Err(err) = state.delete_deck(id) {
            // Translators: a toast: what is before ":" is its title, the rest is behind Details. {error} is why, in English.
            self.error_toast(&with_error(
                &gettext("The deck could not be deleted: {error}"),
                &err,
            ));
            return;
        }
        self.reload_or_toast(Focus::Index(index));

        // The deck was just chosen; short, so it fits beside Undo at 360 px
        // (ADR 0058).
        let title = gettext("Deck deleted");
        let id = id.to_owned();
        let toast = undo_toast(
            &title,
            glib::clone!(
                #[weak(rename_to = page)]
                self,
                move || page.undelete(&id, &name)
            ),
        );
        show_toast_from(self, toast);
    }

    /// Merge Into…: asks for the deck to merge `id` into, merges at once,
    /// and the row leaves the list (the focus on the row now at its place,
    /// `index_after_removal`); the toast's Undo brings the deck back (#103,
    /// ADR 0066).
    fn merge(&self, id: &str) {
        let Some(state) = self.imp().state().cloned() else {
            return;
        };
        let id = id.to_owned();
        glib::spawn_future_local(glib::clone!(
            #[weak(rename_to = page)]
            self,
            async move {
                let Some(index) = page.rows().iter().position(|row| row.deck_id() == id) else {
                    return;
                };
                // The dialog gives the focus back to the widget that had it
                // (the row's menu button, gone with the rebuilt list) when
                // it closes, after the answer: the focus is set again then.
                let merged_at: Rc<Cell<Option<usize>>> = Rc::default();
                let closed = glib::clone!(
                    #[weak]
                    page,
                    #[strong]
                    merged_at,
                    move || {
                        if let Some(index) = merged_at.take() {
                            page.focus_index(index);
                        }
                    }
                );
                let Some(done) = merge_deck::run(&page, &state, &id, closed).await else {
                    return;
                };
                page.reload_or_toast(Focus::Index(index));
                merged_at.set(Some(index));
                page.show_merged_toast(&state, done);
            }
        ));
    }

    /// Shows "Decks merged" with Undo, which restores the merged deck and
    /// reads every list again
    /// (`RappelWindow::deck_unmerged`), from whichever page is showing
    /// then. This page is the window's root and lives as long as it, so a
    /// failed Undo is told from here.
    pub fn show_merged_toast(&self, state: &AppState, done: merge_deck::Done) {
        let window = self
            .root()
            .and_downcast::<RappelWindow>()
            .map(|w| w.downgrade());
        let page = self.downgrade();
        let toast = merge_deck::merged_toast(state, done, move |result, done| match result {
            Ok(deck) => {
                if let Some(window) = window.as_ref().and_then(glib::WeakRef::upgrade) {
                    window.deck_unmerged(&deck.id);
                }
            }
            Err(err) => {
                if let Some(page) = page.upgrade() {
                    page.error_message(&merge_deck::not_restored_text(
                        &done.source,
                        &done.target,
                        &err,
                    ));
                }
            }
        });
        show_toast_from(self, toast);
    }

    /// Lists the decks again after a merge was undone, with the focus on
    /// the deck `id` brought back when this page shows; under another page
    /// only the list is read again (`shown` keeps the focus as it was).
    pub fn deck_restored(&self, id: &str) {
        if self.is_on_top() {
            self.reload_or_toast(Focus::Deck(id));
        } else {
            self.refresh();
        }
    }

    /// Makes deck `id`'s row take the focus the next time this page shows,
    /// e.g. the deck a deck page was merged into, going back from it.
    pub fn focus_on_return(&self, id: &str) {
        self.imp().opened.replace(Some(id.to_owned()));
    }

    /// Focuses the row now at `index` after a row there left the list, else
    /// the last one, else the empty state's button.
    fn focus_index(&self, index: usize) {
        let rows = self.rows();
        match index_after_removal(index, rows.len()) {
            Some(i) => {
                rows[i].grab_focus();
            }
            None => {
                self.imp().empty_new_button.grab_focus();
            }
        }
    }

    fn undelete(&self, id: &str, name: &str) {
        let Some(state) = self.imp().state() else {
            return;
        };
        match state.undelete_deck(id) {
            Ok(_) => self.reload_or_toast(Focus::Deck(id)),
            Err(err) => self.error_message(&crate::ui::text::not_restored(name, &err)),
        }
    }

    /// Shows the name dialog unless one is already open.
    async fn ask(&self, purpose: Purpose, decks: Vec<Deck>) -> Option<String> {
        let asking = &self.imp().asking;
        if asking.replace(true) {
            return None;
        }
        let name = deck_name_dialog::ask(self, purpose, decks).await;
        asking.set(false);
        name
    }

    /// The live decks, archived ones included, for the name dialog's
    /// check (an archived deck's name stays taken, ADR 0068), or `None`
    /// after showing a toast.
    fn named_decks(&self, state: &AppState) -> Option<Vec<Deck>> {
        match state.decks_with_archived() {
            Ok(decks) => Some(decks),
            Err(err) => {
                // Translators: a toast: what is before ":" is its title, the rest is behind Details. {error} is why, in English.
                self.error_toast(&with_error(
                    &gettext("The decks could not be read: {error}"),
                    &err,
                ));
                None
            }
        }
    }

    /// The live decks that are not archived, in the list's order, or
    /// `None` after showing a toast. Without their counts: those grow with
    /// the number of cards, and only the rows show them (issue #42).
    fn decks(&self, state: &AppState) -> Option<Vec<Deck>> {
        match state.decks() {
            Ok(decks) => Some(decks),
            Err(err) => {
                // Translators: a toast: what is before ":" is its title, the rest is behind Details. {error} is why, in English.
                self.error_toast(&with_error(
                    &gettext("The decks could not be read: {error}"),
                    &err,
                ));
                None
            }
        }
    }

    /// Reads the decks and shows them, then moves the focus. With
    /// [`Focus::Keep`] and the same decks in the same order, the rows are
    /// updated in place, so a refresh never takes the focus from a row;
    /// otherwise the list is rebuilt and a focused deck keeps the focus.
    fn reload(&self, focus: Focus<'_>) -> rappel_core::Result<()> {
        let imp = self.imp();
        let Some(state) = imp.state() else {
            return Ok(());
        };
        let now = rappel_core::time::now_ms();
        let decks = state.list_decks(now)?;
        let next_day = state.next_day_start(now)?;
        let archived = state.archived_decks()?.len();
        self.arm_refresh(refresh_delay_ms(&decks, now, next_day));
        self.show_archived_count(archived);

        let rows = self.rows();
        let same_decks = rows.len() == decks.len()
            && rows
                .iter()
                .zip(&decks)
                .all(|(row, summary)| row.deck_id() == summary.deck.id);
        let plain: Vec<Deck> = decks.iter().map(|summary| summary.deck.clone()).collect();
        if matches!(focus, Focus::Keep) && same_decks {
            for (row, summary) in rows.iter().zip(&decks) {
                row.update(summary, now, next_day);
                row.set_can_merge(merge_deck::offered(&plain, &summary.deck.id));
            }
            imp.content_stack
                .set_visible_child_name(if decks.is_empty() { "empty" } else { "list" });
            return Ok(());
        }
        // Rebuilt under `Keep` (decks added or removed meanwhile, e.g. by
        // an import): the deck that had the focus keeps it.
        let focused = rows
            .iter()
            .find(|row| row.has_focus() || row.focus_child().is_some())
            .map(DeckRow::deck_id);
        let focus = match (focus, focused.as_deref()) {
            (Focus::Keep, Some(id)) => Focus::Deck(id),
            (focus, _) => focus,
        };

        imp.deck_list.remove_all();
        let mut focus_row: Option<DeckRow> = None;
        for (i, summary) in decks.iter().enumerate() {
            let row = DeckRow::new(summary, now, next_day);
            row.set_can_merge(merge_deck::offered(&plain, &summary.deck.id));
            let wanted = match focus {
                Focus::Keep => false,
                Focus::Deck(id) => summary.deck.id == id,
                Focus::Index(n) => index_after_removal(n, decks.len()) == Some(i),
            };
            if wanted {
                focus_row = Some(row.clone());
            }
            imp.deck_list.append(&row);
        }
        if decks.is_empty() {
            imp.content_stack.set_visible_child_name("empty");
            if !matches!(focus, Focus::Keep) {
                imp.empty_new_button.grab_focus();
            }
        } else {
            imp.content_stack.set_visible_child_name("list");
            if let Some(row) = focus_row {
                row.grab_focus();
            }
        }
        Ok(())
    }

    /// Shows the "Archived Decks" rows while `archived` decks are archived,
    /// with their count, and says in the empty state whether every deck is
    /// archived.
    fn show_archived_count(&self, archived: usize) {
        let imp = self.imp();
        let count = archived_text::row_count(archived);
        let description = archived_text::row_description(archived);
        for (shown, row, label) in [
            (
                imp.archived_list.upcast_ref::<gtk::Widget>(),
                &imp.archived_row,
                &imp.archived_count,
            ),
            (
                imp.archived_empty_clamp.upcast_ref(),
                &imp.archived_empty_row,
                &imp.archived_empty_count,
            ),
        ] {
            shown.set_visible(archived > 0);
            label.set_label(&count);
            // libadwaita-rs lists `Accessible` for `AdwActionRow` only with
            // its `gtk_v4_10` feature, which we do not enable; every widget
            // is one.
            row.upcast_ref::<gtk::Widget>()
                .update_property(&[gtk::accessible::Property::Description(&description)]);
        }
        let (title, text) = archived_text::deck_list_empty(archived);
        imp.empty_status.set_title(&title);
        imp.empty_status.set_description(Some(&text));
    }

    /// Whether the "Archived Decks" row shows, for tests.
    #[cfg(test)]
    pub(crate) fn archived_row_shown(&self) -> bool {
        let imp = self.imp();
        if imp.content_stack.visible_child_name().as_deref() == Some("empty") {
            imp.archived_empty_clamp.get_visible()
        } else {
            imp.archived_list.get_visible()
        }
    }

    /// The rows of the list, in order, for tests.
    #[cfg(test)]
    pub(crate) fn deck_rows(&self) -> Vec<DeckRow> {
        self.rows()
    }

    /// The rows of the list, in order.
    fn rows(&self) -> Vec<DeckRow> {
        let list = &self.imp().deck_list;
        (0..)
            .map_while(|i| list.row_at_index(i))
            .filter_map(|row| row.downcast::<DeckRow>().ok())
            .collect()
    }

    /// [`Self::reload`], with a failure in the page's [`ErrorSlot`]: the
    /// list is read again on every activation of the window and by a
    /// timer, so a lasting failure replaces its toast instead of piling
    /// them up, and a later success takes it away.
    fn reload_or_toast(&self, focus: Focus<'_>) {
        match self.reload(focus) {
            // A "could not be read" toast from an earlier try is stale now.
            Ok(()) => self.imp().read_errors.clear(),
            Err(err) => self.imp().read_errors.show(
                self,
                "reading the decks",
                // Translators: a toast: what is before ":" is its title, the rest is behind Details. {error} is why, in English.
                &with_error(&gettext("The decks could not be read: {error}"), &err),
            ),
        }
    }

    fn error_toast(&self, message: &str) {
        show_error(self, "a deck operation", message);
    }

    /// [`Self::error_toast`] for a message whose title was decided on its
    /// template (a deck's name in it never splits the title).
    fn error_message(&self, message: &Message) {
        show_error_message(self, "a deck operation", message);
    }
}

/// How late the first frame after a deck's push may come and still be
/// logged as its opening time.
const FIRST_FRAME_LOG_LIMIT: Duration = Duration::from_secs(5);

/// Logs (at debug level) how long opening a deck took: building its page,
/// and until the first frame painted after the push (issue #42: a deck of
/// 5,000 cards must open fast). `after-paint` is emitted by the frame clock
/// once per frame, after the frame is drawn (`GdkFrameClock` docs, "Frame
/// phases"); the handler disconnects itself after the first one, and logs
/// nothing if that frame comes more than [`FIRST_FRAME_LOG_LIMIT`] later.
fn log_first_frame(view: &adw::NavigationView, started: Instant, built_ms: u128) {
    let Some(clock) = view.frame_clock() else {
        return;
    };
    let handler: Rc<RefCell<Option<glib::SignalHandlerId>>> = Rc::default();
    let id = clock.connect_after_paint(glib::clone!(
        #[strong]
        handler,
        move |clock| {
            // A frame long after the push (the window was hidden, say) says
            // nothing about opening the deck: disconnect without a log.
            let elapsed = started.elapsed();
            if elapsed < FIRST_FRAME_LOG_LIMIT {
                tracing::debug!(
                    built_ms,
                    first_frame_ms = elapsed.as_millis(),
                    "deck page opened"
                );
            }
            if let Some(id) = handler.take() {
                clock.disconnect(id);
            }
        }
    ));
    handler.replace(Some(id));
}

#[cfg(test)]
mod key_tests {
    use gtk::gdk::{Key, ModifierType as M};

    use super::*;

    /// A deck list with deck "A" holding cards to study, an empty deck "B"
    /// and an archived deck, in a presented navigation view.
    fn list() -> (
        tempfile::TempDir,
        DecksPage,
        gtk::Window,
        adw::NavigationView,
    ) {
        let (tmp, state) = crate::testing::temporary_state();
        let a = state.create_deck("A").unwrap();
        state.create_deck("B").unwrap();
        let old = state.create_deck("Old").unwrap();
        state
            .add_card_with_tags(&a.id, "front", "back", &[])
            .unwrap();
        state.archive_deck(&old.id).unwrap();
        let page: DecksPage = glib::Object::new();
        page.set_state(state).unwrap();
        let view = adw::NavigationView::new();
        let window = gtk::Window::builder()
            .default_width(600)
            .default_height(500)
            .child(&view)
            .build();
        window.present();
        view.push(&page);
        crate::testing::settle();
        (tmp, page, window, view)
    }

    fn reviewing(view: &adw::NavigationView) -> bool {
        view.visible_page().is_some_and(|p| p.is::<ReviewPage>())
    }

    fn row(page: &DecksPage, name: &str) -> DeckRow {
        page.deck_rows()
            .into_iter()
            .find(|row| row.name() == name)
            .unwrap_or_else(|| panic!("no row {name}"))
    }

    /// S on a focused deck row, or on its menu button, starts that deck's
    /// session on the release, once, however many repeats came first
    /// (ADR 0081).
    #[test]
    fn s_on_a_deck_row_starts_its_session_on_release() {
        crate::testing::with_widgets(|| {
            for on_menu_button in [false, true] {
                let (_tmp, page, window, view) = list();
                let a = row(&page, "A");
                if on_menu_button {
                    a.focus_menu_button();
                } else {
                    a.grab_focus();
                }
                for _ in 0..6 {
                    assert_eq!(
                        page.study_key_pressed(Key::s, M::empty()),
                        glib::Propagation::Stop
                    );
                }
                assert!(!reviewing(&view));
                page.study_key_released(Key::s, M::empty());
                assert!(reviewing(&view));
                page.study_key_released(Key::s, M::empty());
                assert_eq!(view.navigation_stack().n_items(), 2);
                window.destroy();
            }
        });
    }

    #[test]
    fn s_does_nothing_elsewhere_with_modifiers_or_for_an_empty_deck() {
        crate::testing::with_widgets(|| {
            let (_tmp, page, window, view) = list();
            // Shift+S, Ctrl+S: not swallowed.
            row(&page, "A").grab_focus();
            for modifiers in [M::SHIFT_MASK, M::CONTROL_MASK, M::ALT_MASK] {
                assert_eq!(
                    page.study_key_pressed(Key::s, modifiers),
                    glib::Propagation::Proceed
                );
            }
            page.study_key_released(Key::s, M::empty());
            assert!(!reviewing(&view));
            // The Archived Decks row is not a deck row.
            assert!(page.archived_row_shown());
            let archived = page.imp().archived_list.row_at_index(0).unwrap();
            archived.grab_focus();
            assert_eq!(
                page.study_key_pressed(Key::s, M::empty()),
                glib::Propagation::Proceed
            );
            page.study_key_released(Key::s, M::empty());
            assert!(!reviewing(&view));
            // A deck with nothing to study starts nothing.
            row(&page, "B").grab_focus();
            let _ = page.study_key_pressed(Key::s, M::empty());
            page.study_key_released(Key::s, M::empty());
            assert!(!reviewing(&view));
            window.destroy();
        });
    }

    fn deck_page_shown(view: &adw::NavigationView) -> bool {
        view.visible_page()
            .is_some_and(|p| p.is::<crate::pages::DeckPage>())
    }

    /// The review's stale hold: S pressed on a row, Return opens the deck
    /// (the row's own binding takes Return, so this page never sees that
    /// press), S released on the deck page; back on the list, Ctrl+S must
    /// start nothing. Hiding the page cleared the hold; Ctrl's press and
    /// the release's modifier would each have refused it too.
    #[test]
    fn a_hold_left_by_opening_a_deck_starts_nothing_later() {
        crate::testing::with_widgets(|| {
            let (_tmp, page, window, view) = list();
            let a = row(&page, "A");
            a.grab_focus();
            assert_eq!(
                page.study_key_pressed(Key::s, M::empty()),
                glib::Propagation::Stop
            );
            // Return on the row: its activation opens the deck page.
            a.activate();
            crate::testing::settle();
            assert!(deck_page_shown(&view));
            // S goes up on the deck page, which never saw it pressed (its
            // own tests cover that): this page does not see that release.
            view.pop();
            crate::testing::settle();
            row(&page, "A").grab_focus();
            // A plain release alone: the hold is gone.
            page.study_key_released(Key::s, M::empty());
            assert!(!reviewing(&view));
            // Ctrl, then Ctrl+S pressed and released.
            let _ = page.study_key_pressed(Key::Control_L, M::empty());
            assert_eq!(
                page.study_key_pressed(Key::s, M::CONTROL_MASK),
                glib::Propagation::Proceed
            );
            page.study_key_released(Key::s, M::CONTROL_MASK);
            assert!(!reviewing(&view));
            assert_eq!(view.navigation_stack().n_items(), 1);
            window.destroy();
        });
    }

    /// Even with the hold still there (no navigation), another key's press
    /// or a modifier on the release refuses it.
    #[test]
    fn ctrl_or_shift_cancel_a_held_s() {
        crate::testing::with_widgets(|| {
            let (_tmp, page, window, view) = list();
            row(&page, "A").grab_focus();
            for (other, release) in [
                (Some(Key::Control_L), M::CONTROL_MASK),
                (Some(Key::Shift_L), M::SHIFT_MASK),
                (None, M::CONTROL_MASK),
            ] {
                let _ = page.study_key_pressed(Key::s, M::empty());
                if let Some(other) = other {
                    assert_eq!(
                        page.study_key_pressed(other, M::empty()),
                        glib::Propagation::Proceed
                    );
                }
                page.study_key_released(Key::s, release);
                assert!(!reviewing(&view), "{other:?} {release:?}");
            }
            // Return on the row's menu button opens its popover; the press
            // is claimed below the page, which sees it only in the capture
            // phase.
            row(&page, "A").focus_menu_button();
            let _ = page.study_key_pressed(Key::s, M::empty());
            page.imp().start_hold.observe_press(Key::Return, M::empty());
            page.study_key_released(Key::s, M::empty());
            assert!(!reviewing(&view));
            // Its own repeats, seen there too, keep the hold.
            let _ = page.study_key_pressed(Key::s, M::empty());
            page.imp().start_hold.observe_press(Key::s, M::empty());
            page.study_key_released(Key::s, M::empty());
            assert!(reviewing(&view));
            window.destroy();
        });
    }

    /// S pressed on one deck's row and released on another's starts
    /// nothing; pressed and released on the same row, it starts that deck.
    #[test]
    fn the_release_must_come_on_the_row_of_the_press() {
        crate::testing::with_widgets(|| {
            let (_tmp, page, window, view) = list();
            let state = page.imp().state().cloned().unwrap();
            let b = row(&page, "B");
            state
                .add_card_with_tags(&b.deck_id(), "front", "back", &[])
                .unwrap();
            row(&page, "A").grab_focus();
            let _ = page.study_key_pressed(Key::s, M::empty());
            row(&page, "B").grab_focus();
            page.study_key_released(Key::s, M::empty());
            assert!(!reviewing(&view));
            let _ = page.study_key_pressed(Key::s, M::empty());
            page.study_key_released(Key::s, M::empty());
            assert!(reviewing(&view));
            window.destroy();
        });
    }

    /// The empty state's button is not a deck row either.
    #[test]
    fn s_on_the_empty_states_button_does_nothing() {
        crate::testing::with_widgets(|| {
            let (_tmp, state) = crate::testing::temporary_state();
            let page: DecksPage = glib::Object::new();
            page.set_state(state).unwrap();
            let view = adw::NavigationView::new();
            let window = gtk::Window::builder().child(&view).build();
            window.present();
            view.push(&page);
            crate::testing::settle();
            page.imp().empty_new_button.grab_focus();
            assert_eq!(
                page.study_key_pressed(Key::s, M::empty()),
                glib::Propagation::Proceed
            );
            page.study_key_released(Key::s, M::empty());
            assert!(!reviewing(&view));
            window.destroy();
        });
    }
}
