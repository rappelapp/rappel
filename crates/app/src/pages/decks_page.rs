//! The root page: the list of decks, or an empty state, with create,
//! rename and delete (undone from a toast). Activating a deck opens its
//! page. Layout in `data/decks_page.blp`.

use adw::prelude::*;
use adw::subclass::prelude::*;
use cards_core::domain::Deck;
use gtk::glib;

use crate::dialogs::deck_name_dialog::{self, Purpose};
use crate::pages::DeckPage;
use crate::state::AppState;
use crate::widgets::DeckRow;
use crate::window::{persistent_toast, show_toast_from};

mod imp {
    use std::cell::{Cell, OnceCell, RefCell};

    use adw::prelude::*;
    use adw::subclass::prelude::*;
    use gtk::glib;

    use crate::state::AppState;

    #[derive(Debug, Default, gtk::CompositeTemplate)]
    #[template(resource = "/io/github/rsmirke/Cards/decks_page.ui")]
    pub struct DecksPage {
        #[template_child]
        pub content_stack: TemplateChild<gtk::Stack>,
        #[template_child]
        pub deck_list: TemplateChild<gtk::ListBox>,
        #[template_child]
        pub empty_new_button: TemplateChild<gtk::Button>,
        /// Set once by the window when the database is open (ADR 0011).
        pub state: OnceCell<AppState>,
        /// A name dialog is open; a second one (double click, Ctrl+N) is
        /// not opened on top of it.
        pub asking: Cell<bool>,
        /// The deck whose page was opened last, focused when coming back.
        pub opened: RefCell<Option<String>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for DecksPage {
        const NAME: &'static str = "CardsDecksPage";
        type Type = super::DecksPage;
        type ParentType = adw::NavigationPage;

        fn class_init(klass: &mut Self::Class) {
            klass.bind_template();
            // Activated by the deck rows' menus with the deck id.
            klass.install_action(
                "decks.rename",
                Some(glib::VariantTy::STRING),
                |page, _, id| {
                    if let Some(id) = id.and_then(glib::Variant::str) {
                        page.rename(id.to_owned());
                    }
                },
            );
            klass.install_action(
                "decks.delete",
                Some(glib::VariantTy::STRING),
                |page, _, id| {
                    if let Some(id) = id.and_then(glib::Variant::str) {
                        page.delete(id);
                    }
                },
            );
        }

        fn instance_init(obj: &glib::subclass::InitializingObject<Self>) {
            obj.init_template();
        }
    }

    impl ObjectImpl for DecksPage {
        fn constructed(&self) {
            self.parent_constructed();
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
            // Back from a deck: its counts may have changed.
            page.connect_shown(|page| {
                if let Some(id) = page.imp().opened.take() {
                    page.reload_or_toast(super::Focus::Deck(&id));
                }
            });
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
    pub fn set_state(&self, state: AppState) -> cards_core::Result<()> {
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

    /// Pushes the page of deck `id`.
    fn open(&self, id: &str) {
        let Some(state) = self.imp().state() else {
            return;
        };
        let Some(deck) = self
            .decks(state)
            .and_then(|d| d.into_iter().find(|d| d.id == id))
        else {
            return;
        };
        let Some(view) = self
            .ancestor(adw::NavigationView::static_type())
            .and_downcast::<adw::NavigationView>()
        else {
            tracing::error!("the deck list is not inside a navigation view");
            return;
        };
        match DeckPage::new(state.clone(), deck) {
            Ok(page) => {
                self.imp().opened.replace(Some(id.to_owned()));
                view.push(&page);
            }
            Err(err) => self.error_toast(&format!("The cards could not be read: {err}")),
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
                let Some(decks) = page.decks(&state) else {
                    return;
                };
                let Some(name) = page.ask(Purpose::Create, decks).await else {
                    return;
                };
                match state.create_deck(&name) {
                    Ok(deck) => page.reload_or_toast(Focus::Deck(&deck.id)),
                    Err(err) => page.error_toast(&format!("The deck could not be created: {err}")),
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
                let Some(decks) = page.decks(&state) else {
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
                    Err(err) => page.error_toast(&format!("The deck could not be renamed: {err}")),
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
            self.error_toast(&format!("The deck could not be deleted: {err}"));
            return;
        }
        self.reload_or_toast(Focus::Index(index));

        let toast = adw::Toast::builder()
            .title(format!("“{name}” deleted"))
            .use_markup(false)
            .button_label("_Undo")
            // Never queued behind a toast that stays until dismissed.
            .priority(adw::ToastPriority::High)
            .build();
        let id = id.to_owned();
        toast.connect_button_clicked(glib::clone!(
            #[weak(rename_to = page)]
            self,
            move |_| page.undelete(&id, &name)
        ));
        show_toast_from(self, toast);
    }

    fn undelete(&self, id: &str, name: &str) {
        let Some(state) = self.imp().state() else {
            return;
        };
        match state.undelete_deck(id) {
            Ok(_) => self.reload_or_toast(Focus::Deck(id)),
            Err(err) => self.error_toast(&format!("“{name}” could not be restored: {err}")),
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

    /// The live decks, or `None` after showing a toast.
    fn decks(&self, state: &AppState) -> Option<Vec<Deck>> {
        match state.list_decks() {
            Ok(decks) => Some(decks.into_iter().map(|s| s.deck).collect()),
            Err(err) => {
                self.error_toast(&format!("The decks could not be read: {err}"));
                None
            }
        }
    }

    /// Rebuilds the list from the store and moves the focus.
    fn reload(&self, focus: Focus<'_>) -> cards_core::Result<()> {
        let imp = self.imp();
        let Some(state) = imp.state() else {
            return Ok(());
        };
        let decks = state.list_decks()?;
        imp.deck_list.remove_all();
        let mut focus_row: Option<DeckRow> = None;
        for (i, summary) in decks.iter().enumerate() {
            let row = DeckRow::new(summary);
            let wanted = match focus {
                Focus::Keep => false,
                Focus::Deck(id) => summary.deck.id == id,
                Focus::Index(n) => i == n.min(decks.len() - 1),
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

    fn reload_or_toast(&self, focus: Focus<'_>) {
        if let Err(err) = self.reload(focus) {
            self.error_toast(&format!("The decks could not be read: {err}"));
        }
    }

    fn error_toast(&self, message: &str) {
        // Deck names are user content: only at debug level in logs (SPEC §2).
        tracing::warn!("a deck operation failed");
        tracing::debug!("{message}");
        show_toast_from(self, persistent_toast(message));
    }
}
