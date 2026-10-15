//! One deck's page: its cards in a `GtkListView`, or an empty state, with
//! add, edit and delete (undone from a toast). Layout in
//! `data/deck_page.blp` (ADR 0013).
//!
//! The list model is a `gio::ListStore` of [`CardObject`] in creation
//! order. Changes update it item by item instead of rebuilding it, so a
//! deck of thousands of cards keeps its scroll position and stays fast.

use std::cmp::Ordering;

use adw::prelude::*;
use adw::subclass::prelude::*;
use cards_core::domain::{Card, Deck};
use gtk::{gio, glib};

use crate::dialogs::card_editor::{CardEditor, Saved};
use crate::state::AppState;
use crate::widgets::{CardObject, CardRow};
use crate::window::{CardsWindow, persistent_toast, show_toast_from};

mod imp {
    use std::cell::{Cell, OnceCell, RefCell};

    use adw::subclass::prelude::*;
    use cards_core::domain::Deck;
    use gtk::{gio, glib};

    use crate::state::AppState;

    #[derive(Debug, Default, gtk::CompositeTemplate)]
    #[template(resource = "/io/github/rsmirke/Cards/deck_page.ui")]
    pub struct DeckPage {
        #[template_child]
        pub content_stack: TemplateChild<gtk::Stack>,
        #[template_child]
        pub card_list: TemplateChild<gtk::ListView>,
        #[template_child]
        pub empty_add_button: TemplateChild<gtk::Button>,
        pub state: OnceCell<AppState>,
        pub deck: OnceCell<Deck>,
        /// The cards, oldest first.
        pub cards: OnceCell<gio::ListStore>,
        /// An editor is open; a second one (Ctrl+N, a click) is not opened.
        pub editing: Cell<bool>,
        /// The card to focus when the editor closes: the one last saved.
        pub focus_after_edit: RefCell<Option<String>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for DeckPage {
        const NAME: &'static str = "CardsDeckPage";
        type Type = super::DeckPage;
        type ParentType = adw::NavigationPage;

        fn class_init(klass: &mut Self::Class) {
            klass.bind_template();
            klass.install_action("deck.new-card", None, |page, _, _| {
                page.new_card();
            });
            // Activated by the card rows' menus with the card id.
            klass.install_action(
                "deck.edit-card",
                Some(glib::VariantTy::STRING),
                |page, _, id| {
                    if let Some(id) = id.and_then(glib::Variant::str) {
                        page.edit(id);
                    }
                },
            );
            klass.install_action(
                "deck.delete-card",
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

    impl ObjectImpl for DeckPage {}
    impl WidgetImpl for DeckPage {}
    impl NavigationPageImpl for DeckPage {}
}

glib::wrapper! {
    pub struct DeckPage(ObjectSubclass<imp::DeckPage>)
        @extends adw::NavigationPage, gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}

impl DeckPage {
    /// The page for `deck`, with its cards loaded.
    ///
    /// # Errors
    /// If the cards cannot be read.
    pub fn new(state: AppState, deck: Deck) -> cards_core::Result<Self> {
        let page: Self = glib::Object::new();
        let imp = page.imp();
        let cards = state.list_cards(&deck.id)?;
        // Deck names are untrusted; NavigationPage:title is plain text.
        page.set_title(&deck.name);

        let store = gio::ListStore::new::<CardObject>();
        let items: Vec<CardObject> = cards.into_iter().map(CardObject::new).collect();
        store.extend_from_slice(&items);
        store.connect_items_changed(glib::clone!(
            #[weak]
            page,
            move |_, _, _, _| page.update_empty_state()
        ));
        imp.card_list
            .set_model(Some(&gtk::NoSelection::new(Some(store.clone()))));
        imp.card_list.set_factory(Some(&row_factory()));
        imp.card_list.connect_activate(glib::clone!(
            #[weak]
            page,
            move |_, position| {
                if let Some(item) = page.item(position) {
                    page.open_editor(Some(&item.card()));
                }
            }
        ));

        // A new object's cells are empty, so these cannot fail.
        let _ = imp.state.set(state);
        let _ = imp.deck.set(deck);
        let _ = imp.cards.set(store);
        page.update_empty_state();
        Ok(page)
    }

    /// Opens the editor for a new card in this deck.
    pub fn new_card(&self) {
        self.open_editor(None);
    }

    fn edit(&self, id: &str) {
        if let Some(item) = self.position(id).and_then(|p| self.item(p)) {
            self.open_editor(Some(&item.card()));
        }
    }

    fn open_editor(&self, card: Option<&Card>) {
        let imp = self.imp();
        let (Some(state), Some(deck)) = (imp.state.get(), imp.deck.get()) else {
            return;
        };
        if imp.editing.replace(true) {
            return;
        }
        let editor = CardEditor::new(
            state.clone(),
            &deck.id,
            card,
            glib::clone!(
                #[weak(rename_to = page)]
                self,
                move |saved| page.saved(saved)
            ),
        );
        editor.connect_closed(glib::clone!(
            #[weak(rename_to = page)]
            self,
            move |_| {
                let imp = page.imp();
                imp.editing.set(false);
                if let Some(id) = imp.focus_after_edit.take()
                    && let Some(position) = page.position(&id)
                {
                    page.focus(position);
                }
            }
        ));
        editor.show(self);
    }

    /// Puts a saved card into the list.
    fn saved(&self, saved: Saved) {
        let Some(store) = self.imp().cards.get() else {
            return;
        };
        let card = match saved {
            Saved::Added(card) => {
                insert_in_order(store, &CardObject::new(card.clone()));
                card
            }
            Saved::Edited(card) => {
                match self.position(&card.id) {
                    Some(position) => store.splice(position, 1, &[CardObject::new(card.clone())]),
                    None => {
                        insert_in_order(store, &CardObject::new(card.clone()));
                    }
                }
                card
            }
        };
        self.imp().focus_after_edit.replace(Some(card.id));
    }

    /// Deletes at once; the toast's Undo brings the card back.
    fn delete(&self, id: &str) {
        let imp = self.imp();
        let (Some(state), Some(store)) = (imp.state.get(), imp.cards.get()) else {
            return;
        };
        let Some(position) = self.position(id) else {
            return;
        };
        if let Err(err) = state.delete_card(id) {
            self.error_toast(&format!("The card could not be deleted: {err}"));
            return;
        }
        store.remove(position);
        if store.n_items() == 0 {
            imp.empty_add_button.grab_focus();
        } else {
            self.focus(position.min(store.n_items() - 1));
        }

        // The front is not in the title: it can be long or many lines.
        let toast = adw::Toast::builder()
            .title("Card deleted")
            .use_markup(false)
            .button_label("_Undo")
            // Never queued behind a toast that stays until dismissed.
            .priority(adw::ToastPriority::High)
            .build();
        // Undo must work after going back to the deck list too, when this
        // page may be gone: the closure keeps the state, not the page.
        let id = id.to_owned();
        let state = state.clone();
        let page = self.downgrade();
        let window = self
            .root()
            .and_downcast::<CardsWindow>()
            .map(|w| w.downgrade());
        toast.connect_button_clicked(move |_| {
            let window = window.as_ref().and_then(glib::WeakRef::upgrade);
            match state.undelete_card(&id) {
                Ok(card) => {
                    if let Some(page) = page.upgrade() {
                        page.restored(&card);
                    }
                    if let Some(window) = window {
                        // The deck may have been reopened in a new page.
                        window.card_restored(&card);
                    }
                }
                Err(err) => {
                    let message = format!("The card could not be restored: {err}");
                    tracing::warn!("a card operation failed");
                    tracing::debug!("{message}");
                    if let Some(window) = window {
                        window.add_toast(persistent_toast(&message));
                    }
                }
            }
        });
        show_toast_from(self, toast);
    }

    /// Puts an undeleted card back in its place and focuses it, if it is
    /// a card of this deck. Harmless when it is already there.
    pub fn restored(&self, card: &Card) {
        let imp = self.imp();
        let (Some(store), Some(deck)) = (imp.cards.get(), imp.deck.get()) else {
            return;
        };
        if card.deck_id != deck.id {
            return;
        }
        let position = match self.position(&card.id) {
            Some(position) => position,
            None => insert_in_order(store, &CardObject::new(card.clone())),
        };
        self.focus(position);
    }

    /// Scrolls to the card at `position` and gives it the keyboard focus.
    fn focus(&self, position: u32) {
        self.imp()
            .card_list
            .scroll_to(position, gtk::ListScrollFlags::FOCUS, None);
    }

    fn update_empty_state(&self) {
        let imp = self.imp();
        let empty = imp.cards.get().is_none_or(|s| s.n_items() == 0);
        imp.content_stack
            .set_visible_child_name(if empty { "empty" } else { "list" });
    }

    fn item(&self, position: u32) -> Option<CardObject> {
        self.imp().cards.get()?.item(position).and_downcast()
    }

    /// Where card `id` is in the list. A linear scan: a few microseconds
    /// for thousands of cards, and only on a user action.
    fn position(&self, id: &str) -> Option<u32> {
        let store = self.imp().cards.get()?;
        (0..store.n_items()).find(|&position| {
            store
                .item(position)
                .and_downcast::<CardObject>()
                .is_some_and(|c| c.card().id == id)
        })
    }

    fn error_toast(&self, message: &str) {
        // The message may hold card content: only at debug level (SPEC §2).
        tracing::warn!("a card operation failed");
        tracing::debug!("{message}");
        show_toast_from(self, persistent_toast(message));
    }
}

/// Inserts `item` where its creation order puts it and returns that
/// position. Card ids increase with creation (ADR 0008), the same order as
/// `list_cards`. `insert_sorted` searches by halves, so it relies on that:
/// an import that inserts cards with older ids (SPEC §4.1) must keep `seq`
/// order equal to id order, or this must compare by `seq` (ADR 0013).
fn insert_in_order(store: &gio::ListStore, item: &CardObject) -> u32 {
    store.insert_sorted(item, |a, b| {
        match (
            a.downcast_ref::<CardObject>(),
            b.downcast_ref::<CardObject>(),
        ) {
            (Some(a), Some(b)) => a.card().id.cmp(&b.card().id),
            _ => Ordering::Equal,
        }
    })
}

/// Builds a [`CardRow`] per visible row and points it at its card.
fn row_factory() -> gtk::SignalListItemFactory {
    let factory = gtk::SignalListItemFactory::new();
    factory.connect_setup(|_, item| {
        if let Some(item) = item.downcast_ref::<gtk::ListItem>() {
            item.set_child(Some(&CardRow::default()));
        }
    });
    factory.connect_bind(|_, item| {
        let Some(item) = item.downcast_ref::<gtk::ListItem>() else {
            return;
        };
        if let (Some(card), Some(row)) = (
            item.item().and_downcast::<CardObject>(),
            item.child().and_downcast::<CardRow>(),
        ) {
            row.bind(&card);
            // Screen readers read the front, not "row".
            item.set_accessible_label(&card.preview());
        }
    });
    factory.connect_unbind(|_, item| {
        if let Some(row) = item
            .downcast_ref::<gtk::ListItem>()
            .and_then(|item| item.child().and_downcast::<CardRow>())
        {
            row.unbind();
        }
    });
    factory
}
