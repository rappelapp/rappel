//! One row of the deck page's card list: the front on one line and a menu
//! with Edit, Reset Progress and Delete. Layout in `data/card_row.blp`.
//!
//! Rows are recycled by the `GtkListView`: [`CardRow::bind`] points a row
//! at another card. As in `DeckRow`, the menu items activate the row's own
//! `row.edit` / `row.reset` / `row.delete` actions, which forward to the
//! page's `deck.edit-card` / `deck.reset-card` / `deck.delete-card` with the bound card's id.

use adw::subclass::prelude::*;
use gtk::glib;

use super::CardObject;
use crate::ui::rows;

mod imp {
    use std::cell::RefCell;

    use adw::subclass::prelude::*;
    use gtk::glib;

    #[derive(Debug, Default, gtk::CompositeTemplate)]
    #[template(resource = "/io/github/rsmirke/Rappel/card_row.ui")]
    pub struct CardRow {
        #[template_child]
        pub front_label: TemplateChild<gtk::Label>,
        #[template_child]
        pub menu_button: TemplateChild<gtk::MenuButton>,
        /// The bound card, empty while unbound.
        pub card_id: RefCell<String>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for CardRow {
        const NAME: &'static str = "RappelCardRow";
        type Type = super::CardRow;
        type ParentType = gtk::Box;

        fn class_init(klass: &mut Self::Class) {
            klass.bind_template();
            klass.install_action("row.edit", None, |row, _, _| {
                row.forward("deck.edit-card");
            });
            klass.install_action("row.reset", None, |row, _, _| {
                row.forward("deck.reset-card");
            });
            klass.install_action("row.delete", None, |row, _, _| {
                row.forward("deck.delete-card");
            });
        }

        fn instance_init(obj: &glib::subclass::InitializingObject<Self>) {
            obj.init_template();
        }
    }

    impl ObjectImpl for CardRow {}
    impl WidgetImpl for CardRow {}
    impl BoxImpl for CardRow {}
}

glib::wrapper! {
    pub struct CardRow(ObjectSubclass<imp::CardRow>)
        @extends gtk::Box, gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget, gtk::Orientable;
}

impl Default for CardRow {
    fn default() -> Self {
        glib::Object::new()
    }
}

impl CardRow {
    /// Shows `item` in this row.
    pub fn bind(&self, item: &CardObject) {
        let imp = self.imp();
        let preview = item.preview();
        imp.front_label.set_label(&preview);
        rows::label_menu_button(&imp.menu_button, &preview);
        imp.card_id.replace(item.card().id.clone());
    }

    /// Forgets the card, before the row is recycled.
    pub fn unbind(&self) {
        self.imp().card_id.borrow_mut().clear();
    }

    fn forward(&self, action: &str) {
        let id = self.imp().card_id.borrow().clone();
        if id.is_empty() {
            return;
        }
        rows::forward_id(self, action, &id);
    }
}
