//! `CardObject`: one card as a list model item, for the deck page's
//! `GtkListView` (ADR 0013, 0042). It holds the card, its tags (each name on
//! one line, for the pills), the one-line previews of its front and back
//! (plain text, never markup), and whether it is chosen in selection mode
//! (the `selected` property, which a row widget follows, ADR 0065). The previews are made the first time a row
//! shows the card and then kept, so opening a deck of 5,000 cards parses
//! nothing until the rows scroll into view, and binding a recycled row
//! again is cheap.

use std::cell::{Cell, OnceCell, Ref, RefCell};

use gtk::glib;
use gtk::subclass::prelude::*;
use rappel_core::content::plain_summary;
use rappel_core::domain::{Card, Tag, TaggedCard};

use super::card_row::text::row_label;
use crate::ui::text::one_line;

mod imp {
    use super::{Card, Cell, OnceCell, RefCell, Tag};
    use gtk::glib;
    use gtk::glib::prelude::*;
    use gtk::subclass::prelude::*;

    #[derive(Debug, glib::Properties)]
    #[properties(wrapper_type = super::CardObject)]
    pub struct CardObject {
        pub card: RefCell<Card>,
        /// The tag names, each on one line, for the pills.
        pub tags: RefCell<Vec<String>>,
        /// The tags themselves, for Remove Tag… in selection mode.
        pub tag_list: RefCell<Vec<Tag>>,
        /// Chosen in selection mode.
        #[property(get, set)]
        pub selected: Cell<bool>,
        pub front: OnceCell<String>,
        pub back: OnceCell<String>,
        pub is_new: Cell<bool>,
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
                    suspended: false,
                }),
                tags: RefCell::default(),
                tag_list: RefCell::default(),
                selected: Cell::new(false),
                front: OnceCell::new(),
                back: OnceCell::new(),
                is_new: Cell::new(false),
            }
        }
    }

    #[glib::object_subclass]
    impl ObjectSubclass for CardObject {
        const NAME: &'static str = "RappelCardObject";
        type Type = super::CardObject;
    }

    #[glib::derived_properties]
    impl ObjectImpl for CardObject {}
}

glib::wrapper! {
    pub struct CardObject(ObjectSubclass<imp::CardObject>);
}

impl CardObject {
    pub fn new(tagged: TaggedCard) -> Self {
        let TaggedCard { card, tags } = tagged;
        let obj: Self = glib::Object::new();
        obj.imp()
            .tags
            .replace(tags.iter().map(|tag| one_line(&tag.name)).collect());
        obj.imp().tag_list.replace(tags);
        obj.imp().card.replace(card);
        obj
    }

    /// The card. An edit replaces the whole object in the model.
    pub fn card(&self) -> Ref<'_, Card> {
        self.imp().card.borrow()
    }

    /// Whether the card is new: no surviving review since it was created or
    /// last reset. Decides whether its row offers Reset Progress. A card
    /// starts as not new; the page sets it when it puts the card in the
    /// list, and replaces the object when it changes, so the row rebinds.
    pub fn is_new(&self) -> bool {
        self.imp().is_new.get()
    }

    /// Marks the card new or not, before the object is put in the list.
    #[must_use]
    pub fn with_new(self, is_new: bool) -> Self {
        self.imp().is_new.set(is_new);
        self
    }

    /// A copy of this item, new or not: a new object, so the list rebinds
    /// its row. The previews already made, and whether it is chosen, are
    /// kept.
    pub fn copy_with_new(&self, is_new: bool) -> Self {
        let imp = self.imp();
        let obj: Self = glib::Object::new();
        let copy = obj.imp();
        copy.card.replace(imp.card.borrow().clone());
        copy.tags.replace(imp.tags.borrow().clone());
        copy.tag_list.replace(imp.tag_list.borrow().clone());
        copy.selected.set(imp.selected.get());
        for (from, to) in [(&imp.front, &copy.front), (&imp.back, &copy.back)] {
            if let Some(text) = from.get() {
                to.get_or_init(|| text.clone());
            }
        }
        copy.is_new.set(is_new);
        obj
    }

    /// The front on one line, without Markdown syntax (see
    /// [`plain_summary`]).
    pub fn front_preview(&self) -> &str {
        self.imp()
            .front
            .get_or_init(|| plain_summary(&self.imp().card.borrow().front_md))
    }

    /// The back on one line, without Markdown syntax; empty for an empty
    /// back.
    pub fn back_preview(&self) -> &str {
        self.imp()
            .back
            .get_or_init(|| plain_summary(&self.imp().card.borrow().back_md))
    }

    /// The tag names, each on one line (`ui::text::one_line`).
    pub fn tags(&self) -> Ref<'_, Vec<String>> {
        self.imp().tags.borrow()
    }

    /// The tags themselves, with their ids (a row's menu studies them).
    pub fn tag_list(&self) -> Ref<'_, Vec<Tag>> {
        self.imp().tag_list.borrow()
    }

    /// Whether either preview has been made (they are made on first use).
    #[cfg(test)]
    pub fn has_previews(&self) -> bool {
        self.imp().front.get().is_some() || self.imp().back.get().is_some()
    }

    /// What a screen reader says for the row (see [`row_label`]).
    pub fn row_label(&self) -> String {
        row_label(
            self.front_preview(),
            self.back_preview(),
            &self.tags(),
            self.card().suspended,
        )
    }
}

impl crate::ui::selection::SelectableCard for CardObject {
    fn card_id(&self) -> String {
        self.card().id.clone()
    }

    fn card_suspended(&self) -> bool {
        self.card().suspended
    }

    fn card_is_new(&self) -> bool {
        self.is_new()
    }

    fn card_tags(&self) -> Vec<Tag> {
        self.imp().tag_list.borrow().clone()
    }

    fn set_chosen(&self, chosen: bool) {
        self.set_selected(chosen);
    }
}
