//! `CardObject`: one card as a list model item, for the deck page's
//! `GtkListView` (ADR 0013). It holds the card and its one-line preview
//! (plain text, never markup), computed once, so binding a recycled row is
//! cheap.

use std::cell::{Ref, RefCell};

use cards_core::content::plain_summary;
use cards_core::domain::Card;
use gtk::glib;
use gtk::subclass::prelude::*;

mod imp {
    use super::{Card, RefCell};
    use gtk::glib;
    use gtk::subclass::prelude::*;

    #[derive(Debug)]
    pub struct CardObject {
        pub card: RefCell<Card>,
        pub preview: RefCell<String>,
    }

    /// `GObject` needs a default; [`super::CardObject::new`], the only
    /// constructor, replaces this empty card at once.
    impl Default for CardObject {
        fn default() -> Self {
            Self {
                card: RefCell::new(Card {
                    id: String::new(),
                    deck_id: String::new(),
                    front_md: String::new(),
                    back_md: String::new(),
                    created_at: 0,
                    updated_at: 0,
                }),
                preview: RefCell::new(String::new()),
            }
        }
    }

    #[glib::object_subclass]
    impl ObjectSubclass for CardObject {
        const NAME: &'static str = "CardsCardObject";
        type Type = super::CardObject;
    }

    impl ObjectImpl for CardObject {}
}

glib::wrapper! {
    pub struct CardObject(ObjectSubclass<imp::CardObject>);
}

impl CardObject {
    pub fn new(card: Card) -> Self {
        let obj: Self = glib::Object::new();
        obj.imp().preview.replace(plain_summary(&card.front_md));
        obj.imp().card.replace(card);
        obj
    }

    /// The card. An edit replaces the whole object in the model.
    pub fn card(&self) -> Ref<'_, Card> {
        self.imp().card.borrow()
    }

    /// The front on one line, without Markdown syntax (see
    /// [`plain_summary`]).
    pub fn preview(&self) -> Ref<'_, String> {
        self.imp().preview.borrow()
    }
}
