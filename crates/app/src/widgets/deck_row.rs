//! One deck in the deck list: name, new/due counts and a menu with Rename
//! and Delete. Layout in `data/deck_row.blp`.
//!
//! The menu items activate the row's own `row.rename` / `row.delete`
//! actions, which forward to the page's `decks.rename` / `decks.delete`
//! with this row's deck id (see `ui::rows`).

use adw::prelude::*;
use adw::subclass::prelude::*;
use gtk::glib;
use rappel_core::domain::DeckSummary;

use crate::ui::rows;

mod imp {
    use std::cell::RefCell;

    use adw::subclass::prelude::*;
    use gtk::glib;

    #[derive(Debug, Default, gtk::CompositeTemplate)]
    #[template(resource = "/io/github/rsmirke/Rappel/deck_row.ui")]
    pub struct DeckRow {
        #[template_child]
        pub menu_button: TemplateChild<gtk::MenuButton>,
        pub deck_id: RefCell<String>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for DeckRow {
        const NAME: &'static str = "RappelDeckRow";
        type Type = super::DeckRow;
        type ParentType = adw::ActionRow;

        fn class_init(klass: &mut Self::Class) {
            klass.bind_template();
            klass.install_action("row.rename", None, |row, _, _| {
                row.forward("decks.rename");
            });
            klass.install_action("row.delete", None, |row, _, _| {
                row.forward("decks.delete");
            });
        }

        fn instance_init(obj: &glib::subclass::InitializingObject<Self>) {
            obj.init_template();
        }
    }

    impl ObjectImpl for DeckRow {}
    impl WidgetImpl for DeckRow {}
    impl ListBoxRowImpl for DeckRow {}
    impl PreferencesRowImpl for DeckRow {}
    impl ActionRowImpl for DeckRow {}
}

glib::wrapper! {
    pub struct DeckRow(ObjectSubclass<imp::DeckRow>)
        @extends adw::ActionRow, adw::PreferencesRow, gtk::ListBoxRow, gtk::Widget,
        @implements gtk::Accessible, gtk::Actionable, gtk::Buildable, gtk::ConstraintTarget;
}

impl DeckRow {
    pub fn new(summary: &DeckSummary) -> Self {
        let row: Self = glib::Object::new();
        let name = &summary.deck.name;
        row.set_title(name);
        row.set_subtitle(&format!(
            "{} new · {} due",
            summary.new_count, summary.due_count
        ));
        rows::label_menu_button(&row.imp().menu_button, name);
        row.imp().deck_id.replace(summary.deck.id.clone());
        row
    }

    /// The id of the deck this row shows.
    pub fn deck_id(&self) -> String {
        self.imp().deck_id.borrow().clone()
    }

    fn forward(&self, action: &str) {
        rows::forward_id(self, action, &self.deck_id());
    }
}
