//! One row of the All Cards list (ADR 0036): the front, on wide rows the
//! back, a caption (deck, status, tags), a check button while cards are
//! being chosen, and a menu. Layout in `data/browser_row.blp`.
//!
//! Rows are recycled by the `GtkListView`: [`BrowserRow::bind`] points a row
//! at another [`BrowseItem`]. The page binds the row's `compact` and
//! `selecting` properties to its own, so a narrow window or the selection
//! mode redraws the rows that exist. As for `CardRow`, the menu items
//! activate the row's own `row.*` actions, which forward to the page's
//! `browser.*` actions with the bound card's id.

use adw::prelude::*;
use adw::subclass::prelude::*;
use gtk::glib;

use super::BrowseItem;
use crate::pages::browser_text as text;
use crate::ui::rows;

mod imp {
    use std::cell::{Cell, RefCell};

    use adw::subclass::prelude::*;
    use gtk::glib;
    use gtk::glib::prelude::*;
    use gtk::prelude::*;

    use crate::widgets::BrowseItem;

    #[derive(Debug, Default, gtk::CompositeTemplate, glib::Properties)]
    #[properties(wrapper_type = super::BrowserRow)]
    #[template(resource = "/io/github/rsmirke/Rappel/browser_row.ui")]
    pub struct BrowserRow {
        #[template_child]
        pub check: TemplateChild<gtk::CheckButton>,
        #[template_child]
        pub front_label: TemplateChild<gtk::Label>,
        #[template_child]
        pub back_label: TemplateChild<gtk::Label>,
        #[template_child]
        pub caption_label: TemplateChild<gtk::Label>,
        #[template_child]
        pub menu_button: TemplateChild<gtk::MenuButton>,
        /// A narrow window: no back, no tags.
        #[property(get, set)]
        pub compact: Cell<bool>,
        /// Cards are being chosen: the check button shows, the menu does not.
        #[property(get, set)]
        pub selecting: Cell<bool>,
        /// The bound item, empty while unbound.
        pub item: RefCell<Option<BrowseItem>>,
        pub list_item: glib::WeakRef<gtk::ListItem>,
        pub selected_handler: RefCell<Option<glib::SignalHandlerId>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for BrowserRow {
        const NAME: &'static str = "RappelBrowserRow";
        type Type = super::BrowserRow;
        type ParentType = gtk::Box;

        fn class_init(klass: &mut Self::Class) {
            klass.bind_template();
            klass.install_action("row.edit", None, |row, _, _| {
                row.forward("browser.edit-card");
            });
            klass.install_action("row.info", None, |row, _, _| {
                row.forward("browser.card-info");
            });
            klass.install_action("row.suspend", None, |row, _, _| {
                row.forward("browser.suspend-card");
            });
            klass.install_action("row.unsuspend", None, |row, _, _| {
                row.forward("browser.unsuspend-card");
            });
            klass.install_action("row.reset", None, |row, _, _| {
                row.forward("browser.reset-card");
            });
            klass.install_action("row.delete", None, |row, _, _| {
                row.forward("browser.delete-card");
            });
        }

        fn instance_init(obj: &glib::subclass::InitializingObject<Self>) {
            obj.init_template();
        }
    }

    #[glib::derived_properties]
    impl ObjectImpl for BrowserRow {
        fn constructed(&self) {
            self.parent_constructed();
            let row = self.obj();
            row.connect_compact_notify(super::BrowserRow::refresh);
            row.connect_selecting_notify(super::BrowserRow::refresh);

            // A long press on a touch screen starts choosing cards with
            // this one (a mouse has the Select button and Ctrl+A).
            // Claiming the sequence keeps the release from activating the
            // row (GtkGesture: a claimed sequence cancels the others).
            let press = gtk::GestureLongPress::new();
            press.set_touch_only(true);
            press.connect_pressed(glib::clone!(
                #[weak]
                row,
                move |gesture, _, _| {
                    gesture.set_state(gtk::EventSequenceState::Claimed);
                    row.forward("browser.select-card");
                }
            ));
            row.add_controller(press);
        }
    }
    impl WidgetImpl for BrowserRow {}
    impl BoxImpl for BrowserRow {}
}

glib::wrapper! {
    pub struct BrowserRow(ObjectSubclass<imp::BrowserRow>)
        @extends gtk::Box, gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget, gtk::Orientable;
}

impl Default for BrowserRow {
    fn default() -> Self {
        glib::Object::new()
    }
}

impl BrowserRow {
    /// Shows `item` in this row. `list_item` is the list's item for the
    /// row: its accessible label is what a screen reader says.
    pub fn bind(&self, item: &BrowseItem, list_item: &gtk::ListItem) {
        self.unbind();
        let imp = self.imp();
        imp.list_item.set(Some(list_item));
        imp.item.replace(Some(item.clone()));
        let handler = item.connect_selected_notify(glib::clone!(
            #[weak(rename_to = row)]
            self,
            move |_| row.refresh()
        ));
        imp.selected_handler.replace(Some(handler));
        self.refresh();
    }

    /// Forgets the item, before the row is recycled.
    pub fn unbind(&self) {
        let imp = self.imp();
        if let (Some(item), Some(handler)) = (imp.item.take(), imp.selected_handler.take()) {
            item.disconnect(handler);
        }
        imp.list_item.set(None);
    }

    /// The bound card's id, empty while unbound.
    pub fn card_id(&self) -> String {
        self.imp()
            .item
            .borrow()
            .as_ref()
            .map(BrowseItem::id)
            .unwrap_or_default()
    }

    /// Redraws everything that depends on the item, `compact` or
    /// `selecting`.
    fn refresh(&self) {
        let imp = self.imp();
        let item = imp.item.borrow();
        let Some(item) = item.as_ref() else {
            return;
        };
        let compact = self.compact();
        let selecting = self.selecting();
        let front = item.front_preview();
        let back = item.back_preview();
        let suspended = item.suspended();
        let status = text::status_text(item.row().status, rappel_core::time::now_ms());
        let caption = text::caption(&item.row().deck_name, &status, &item.tags_line(), compact);

        imp.front_label.set_label(front);
        if suspended {
            imp.front_label.add_css_class("dim-label");
        } else {
            imp.front_label.remove_css_class("dim-label");
        }
        imp.back_label.set_label(back);
        imp.back_label.set_visible(!compact && !back.is_empty());
        imp.caption_label.set_label(&caption);
        imp.check.set_visible(selecting);
        imp.check.set_active(item.selected());
        imp.menu_button.set_visible(!selecting);
        rows::label_menu_button(&imp.menu_button, front);
        self.action_set_enabled("row.suspend", !suspended);
        self.action_set_enabled("row.unsuspend", suspended);
        if let Some(list_item) = imp.list_item.upgrade() {
            // The tags are part of the caption only on wide rows; a screen
            // reader always hears them.
            let spoken = text::caption(&item.row().deck_name, &status, &item.tags_line(), false);
            list_item.set_accessible_label(&text::row_label(
                front,
                &spoken,
                selecting.then(|| item.selected()),
            ));
        }
    }

    fn forward(&self, action: &str) {
        let id = self.card_id();
        if id.is_empty() {
            return;
        }
        rows::forward_id(self, action, &id);
    }
}
