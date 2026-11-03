//! One deck in the deck list: name, what it has to study (or when it next
//! will, `text`) and a menu with Practice, then Rename, Merge Into…,
//! Archive and Delete. Layout in `data/deck_row.blp` (ADR 0041, 0066,
//! 0068, 0070).
//!
//! The menu items activate the row's own `row.practice` / `row.rename` /
//! `row.merge` / `row.archive` / `row.delete` actions, which forward to the
//! page's `decks.practice` / `decks.rename` / `decks.merge` /
//! `decks.archive` / `decks.delete` with this row's deck id (see
//! `ui::rows`).

use adw::prelude::*;
use adw::subclass::prelude::*;
use gtk::glib;
use rappel_core::domain::DeckSummary;

use crate::ui::rows;

pub mod text;

mod imp {
    use std::cell::{Cell, RefCell};

    use adw::subclass::prelude::*;
    use gtk::glib;

    #[derive(Debug, Default, gtk::CompositeTemplate)]
    #[template(resource = "/io/github/rsmirke/Rappel/deck_row.ui")]
    pub struct DeckRow {
        #[template_child]
        pub name_label: TemplateChild<gtk::Label>,
        #[template_child]
        pub due_label: TemplateChild<gtk::Label>,
        #[template_child]
        pub separator_label: TemplateChild<gtk::Label>,
        #[template_child]
        pub other_label: TemplateChild<gtk::Label>,
        #[template_child]
        pub menu_button: TemplateChild<gtk::MenuButton>,
        pub deck_id: RefCell<String>,
        /// Merge Into… is enabled (`set_can_merge`).
        pub can_merge: Cell<bool>,
        /// The accessible label last set, "Scratch, 12 due, 5 new".
        pub accessible_label: RefCell<String>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for DeckRow {
        const NAME: &'static str = "RappelDeckRow";
        type Type = super::DeckRow;
        type ParentType = gtk::ListBoxRow;

        fn class_init(klass: &mut Self::Class) {
            klass.bind_template();
            klass.install_action("row.practice", None, |row, _, _| {
                row.forward("decks.practice");
            });
            klass.install_action("row.rename", None, |row, _, _| {
                row.forward("decks.rename");
            });
            klass.install_action("row.merge", None, |row, _, _| {
                row.forward("decks.merge");
            });
            klass.install_action("row.archive", None, |row, _, _| {
                row.forward("decks.archive");
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
}

glib::wrapper! {
    pub struct DeckRow(ObjectSubclass<imp::DeckRow>)
        @extends gtk::ListBoxRow, gtk::Widget,
        @implements gtk::Accessible, gtk::Actionable, gtk::Buildable, gtk::ConstraintTarget;
}

impl DeckRow {
    /// A row for `summary`, its line worded for `now` when the next study
    /// day begins at `next_day` (UTC milliseconds).
    pub fn new(summary: &DeckSummary, now: i64, next_day: i64) -> Self {
        let row: Self = glib::Object::new();
        row.imp().deck_id.replace(summary.deck.id.clone());
        row.update(summary, now, next_day);
        row
    }

    /// Shows `summary` (the same deck, maybe renamed or with other counts)
    /// in place, so a refresh keeps the keyboard focus on the row.
    pub fn update(&self, summary: &DeckSummary, now: i64, next_day: i64) {
        let imp = self.imp();
        let name = &summary.deck.name;
        let line = text::row_text(summary, now, next_day);
        imp.name_label.set_label(name);
        set_part(&imp.due_label, line.due.as_deref());
        set_part(&imp.other_label, line.other.as_deref());
        // Every live card that is not suspended can be practised (ADR 0070).
        let practisable = summary.card_count.saturating_sub(summary.suspended_count);
        let can =
            crate::pages::practice_offered(usize::try_from(practisable).unwrap_or(usize::MAX));
        self.action_set_enabled("row.practice", can);
        imp.separator_label.set_visible(line.has_separator());
        // The row is read as one: "Scratch, 12 due, 5 new" (the "·" and the
        // units in words).
        // Set only when it changed: the list refreshes every minute or so,
        // and a screen reader may announce a changed label of the focused
        // row again.
        let label = line.accessible_label(name);
        if *imp.accessible_label.borrow() != label {
            self.update_property(&[gtk::accessible::Property::Label(&label)]);
            rows::label_menu_button(&imp.menu_button, name);
            imp.accessible_label.replace(label);
        }
    }

    /// Enables Merge Into… in the row's menu (when there is another deck
    /// to merge into, `merge_deck::offered`).
    pub fn set_can_merge(&self, can: bool) {
        self.imp().can_merge.set(can);
        self.action_set_enabled("row.merge", can);
    }

    /// Whether Merge Into… is enabled in the row's menu.
    #[cfg(test)]
    pub fn can_merge(&self) -> bool {
        self.imp().can_merge.get()
    }

    /// The id of the deck this row shows.
    pub fn deck_id(&self) -> String {
        self.imp().deck_id.borrow().clone()
    }

    fn forward(&self, action: &str) {
        rows::forward_id(self, action, &self.deck_id());
    }
}

/// Shows `text` in `label`, or hides the label.
fn set_part(label: &gtk::Label, text: Option<&str>) {
    label.set_label(text.unwrap_or_default());
    label.set_visible(text.is_some());
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Practice in a row's menu is offered while the deck has a live card
    /// that is not suspended (ADR 0070).
    #[test]
    fn practice_is_offered_while_the_deck_has_a_card_to_practise() {
        crate::testing::with_widgets(|| {
            let (_tmp, state) = crate::testing::temporary_state();
            let deck = state.create_deck("Spanish").unwrap();
            let now = rappel_core::time::now_ms();
            let list = gtk::ListBox::new();
            let window = gtk::Window::builder().child(&list).build();
            window.present();
            // The row's menu, as drawn: its Practice item follows the
            // `row.practice` action's enabled state.
            let practice_enabled = |state: &crate::state::AppState| {
                let decks = state.list_decks(now).unwrap();
                let row = DeckRow::new(&decks[0], now, now + 1);
                list.remove_all();
                list.append(&row);
                crate::testing::menu_item_sensitive(&row.imp().menu_button, "Practice")
            };
            assert!(!practice_enabled(&state));
            let card = state
                .add_card_with_tags(&deck.id, "q", "a", &[])
                .unwrap()
                .card
                .id;
            assert!(practice_enabled(&state));
            state.set_suspended(&[card], true).unwrap();
            assert!(!practice_enabled(&state));
            window.destroy();
        });
    }
}
