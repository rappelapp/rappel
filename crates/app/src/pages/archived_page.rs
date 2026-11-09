//! The Archived Decks page (ADR 0068): every archived deck with its card
//! count and an Unarchive button. Opened from the "Archived Decks" row at
//! the end of the deck list. Unarchive brings the deck back to the deck
//! list at once, with an Undo toast; after the last one the page goes back
//! to the deck list, focused on that deck. Layout in
//! `data/archived_page.blp`; the texts are in [`text`].

pub mod text;

use adw::prelude::*;
use adw::subclass::prelude::*;
use gtk::glib;

use crate::state::AppState;
use crate::ui::focus::index_after_removal;
use crate::ui::nav;
use crate::ui::toast::{show_error, show_toast_from, undo_toast};
use crate::window::RappelWindow;

/// The empty state's icon, as in its layout, given back after a short
/// window took it off (`ui::breakpoint::compact_status_page`).
const EMPTY_ICON: &str = "folder-symbolic";

mod imp {
    use std::cell::{OnceCell, RefCell};

    use adw::prelude::*;
    use adw::subclass::prelude::*;
    use gtk::glib;

    use crate::state::AppState;

    #[derive(Debug, Default, gtk::CompositeTemplate)]
    #[template(resource = "/io/github/rappelapp/Rappel/archived_page.ui")]
    pub struct ArchivedPage {
        #[template_child]
        pub empty_short: TemplateChild<adw::Breakpoint>,
        #[template_child]
        pub empty_status: TemplateChild<adw::StatusPage>,
        #[template_child]
        pub content_stack: TemplateChild<gtk::Stack>,
        #[template_child]
        pub archived_list: TemplateChild<gtk::ListBox>,
        pub state: OnceCell<AppState>,
        /// The id of the deck of each row, in order, and each row's
        /// Unarchive button.
        pub rows: RefCell<Vec<(String, gtk::Button)>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for ArchivedPage {
        const NAME: &'static str = "RappelArchivedPage";
        type Type = super::ArchivedPage;
        type ParentType = adw::NavigationPage;

        fn class_init(klass: &mut Self::Class) {
            klass.bind_template();
        }

        fn instance_init(obj: &glib::subclass::InitializingObject<Self>) {
            obj.init_template();
        }
    }

    impl ObjectImpl for ArchivedPage {
        fn constructed(&self) {
            self.parent_constructed();
            // The empty state on a short window (ADR 0058).
            crate::ui::breakpoint::compact_status_page(
                &self.empty_short,
                &self.empty_status,
                super::EMPTY_ICON,
            );
            // The focus goes to the first Unarchive button once the page
            // shows: set earlier, AdwNavigationView focuses Back instead
            // (ui/nav.rs, CLAUDE.md "Focus").
            self.obj().connect_shown(|page| page.focus_index(0));
        }
    }
    impl WidgetImpl for ArchivedPage {}
    impl NavigationPageImpl for ArchivedPage {}
}

glib::wrapper! {
    pub struct ArchivedPage(ObjectSubclass<imp::ArchivedPage>)
        @extends adw::NavigationPage, gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}

impl ArchivedPage {
    /// The page, listing the archived decks as they are now.
    ///
    /// # Errors
    /// If the decks cannot be read.
    pub fn new(state: AppState) -> rappel_core::Result<Self> {
        let page: Self = glib::Object::new();
        crate::ui::init_once(&page.imp().state, state);
        page.reload()?;
        Ok(page)
    }

    /// Reads the archived decks again (after an Undo, an import), or says
    /// why not.
    pub fn refresh(&self) {
        if let Err(err) = self.reload() {
            show_error(
                self,
                "reading the archived decks",
                // Translators: a toast: what is before ":" is its title, the rest is behind Details. {error} is why, in English.
                &crate::ui::text::with_error(
                    &crate::i18n::gettext("The decks could not be read: {error}"),
                    &err,
                ),
            );
        }
    }

    /// The deck ids of the rows, in order.
    pub fn deck_ids(&self) -> Vec<String> {
        self.imp()
            .rows
            .borrow()
            .iter()
            .map(|(id, _)| id.clone())
            .collect()
    }

    /// Rebuilds the rows from the archived decks.
    fn reload(&self) -> rappel_core::Result<()> {
        let imp = self.imp();
        let Some(state) = imp.state.get() else {
            return Ok(());
        };
        let decks = state.archived_decks()?;
        // A row rebuilt under the focus (an Undo, an import) gives it to
        // the row now at its place.
        let focused = imp
            .rows
            .borrow()
            .iter()
            .position(|(_, button)| button.is_focus());
        imp.archived_list.remove_all();
        let mut rows = Vec::with_capacity(decks.len());
        for archived in decks {
            let deck = archived.deck;
            let cards = usize::try_from(archived.card_count).unwrap_or(usize::MAX);
            // Deck names are untrusted: plain text, never markup.
            let row = adw::ActionRow::builder()
                .title(&deck.name)
                .subtitle(text::card_count(cards))
                .use_markup(false)
                .title_lines(2)
                .build();
            let tooltip = text::unarchive_tooltip(&deck.name);
            let button = gtk::Button::builder()
                .label(text::unarchive_label())
                .valign(gtk::Align::Center)
                .tooltip_text(&tooltip)
                .build();
            button.update_property(&[gtk::accessible::Property::Label(&tooltip)]);
            let id = deck.id.clone();
            button.connect_clicked(glib::clone!(
                #[weak(rename_to = page)]
                self,
                move |_| page.unarchive(&id)
            ));
            row.add_suffix(&button);
            imp.archived_list.append(&row);
            rows.push((deck.id, button));
        }
        imp.content_stack
            .set_visible_child_name(if rows.is_empty() { "empty" } else { "list" });
        imp.rows.replace(rows);
        if let Some(index) = focused {
            self.focus_index(index);
        }
        Ok(())
    }

    /// Deck `id`'s Unarchive button, for tests.
    #[cfg(test)]
    pub(crate) fn unarchive_button(&self, id: &str) -> Option<gtk::Button> {
        self.imp()
            .rows
            .borrow()
            .iter()
            .find(|(deck, _)| deck == id)
            .map(|(_, button)| button.clone())
    }

    /// Focuses deck `id`'s Unarchive button, if the deck is listed.
    pub fn focus_deck(&self, id: &str) {
        let rows = self.imp().rows.borrow();
        if let Some((_, button)) = rows.iter().find(|(deck, _)| deck == id) {
            button.grab_focus();
        }
    }

    /// Focuses the Unarchive button of the row now at `index`, else the
    /// last one (`index_after_removal`).
    fn focus_index(&self, index: usize) {
        let rows = self.imp().rows.borrow();
        if let Some(i) = index_after_removal(index, rows.len()) {
            rows[i].1.grab_focus();
        }
    }

    /// Unarchives deck `id`: its row leaves (the focus on the row now at
    /// its place), every page reads its decks again, and a toast offers
    /// Undo. After the last one the page goes back to the deck list, which
    /// focuses that deck.
    pub(crate) fn unarchive(&self, id: &str) {
        let Some(state) = self.imp().state.get().cloned() else {
            return;
        };
        let Some(index) = self.deck_ids().iter().position(|d| d == id) else {
            return;
        };
        let deck = match state.unarchive_deck(id) {
            Ok(deck) => deck,
            Err(err) => {
                show_error(self, "unarchiving a deck", &text::not_unarchived(&err));
                return;
            }
        };
        let window = self.root().and_downcast::<RappelWindow>();
        // Every page, this one included, reads its decks again.
        match &window {
            Some(window) => window.data_changed(),
            None => self.refresh(),
        }
        show_toast_from(
            self,
            unarchived_toast(&state, deck.id.clone(), window.as_ref()),
        );
        if self.imp().rows.borrow().is_empty() {
            if let Some(window) = &window {
                window.focus_deck_on_return(&deck.id);
            }
            nav::pop(self);
        } else {
            self.focus_index(index);
        }
    }
}

/// "Deck unarchived" with Undo, which archives deck `id` again as Archive
/// does (`RappelWindow::rearchive_deck`: a page of the deck opened since
/// is left, the focus goes to the row now at its place). Keeps the window
/// weakly, never a page: the toast outlives this one.
fn unarchived_toast(state: &AppState, id: String, window: Option<&RappelWindow>) -> adw::Toast {
    let state = state.clone();
    let window = window.map(glib::object::ObjectExt::downgrade);
    undo_toast(&text::unarchived_toast(), move || {
        let Some(window) = window.as_ref().and_then(glib::WeakRef::upgrade) else {
            return;
        };
        if let Err(err) = window.rearchive_deck(&state, &id) {
            show_error(&window, "archiving a deck again", &text::not_archived(&err));
        }
    })
}
