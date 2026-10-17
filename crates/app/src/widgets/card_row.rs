//! One row of the deck page's card list (ADR 0042): the front as the title,
//! the back dimmed under it with the tags as pills at the end of that line,
//! an icon when the card is suspended, and a menu with Edit, Card
//! Information, Suspend or Unsuspend, Reset Progress and Delete.
//! Layout in `data/card_row.blp`; what a screen reader says in `text.rs`.
//!
//! Rows are recycled by the `GtkListView`: [`CardRow::bind`] points a row
//! at another card, reusing its pill labels. The `compact` property (bound
//! to the page's, a narrow window) shows fewer pills. As in `DeckRow`, the
//! menu items activate the row's own `row.edit` / `row.info` /
//! `row.suspend` / `row.unsuspend` / `row.reset` / `row.delete` actions,
//! which forward to the page's `deck.edit-card` / `deck.card-info` /
//! `deck.suspend-card` / `deck.unsuspend-card` / `deck.reset-card` /
//! `deck.delete-card` with the bound card's id.

use adw::prelude::*;
use adw::subclass::prelude::*;
use gtk::glib;

use super::CardObject;
use super::tag_pills::text::{COMPACT_ROW_PILLS, ROW_PILLS};
use crate::ui::rows;

pub mod text;

mod imp {
    use std::cell::{Cell, RefCell};

    use adw::prelude::*;
    use adw::subclass::prelude::*;
    use gtk::glib;

    use crate::widgets::TagPills;

    #[derive(Debug, Default, gtk::CompositeTemplate, glib::Properties)]
    #[properties(wrapper_type = super::CardRow)]
    #[template(resource = "/io/github/rsmirke/Rappel/card_row.ui")]
    pub struct CardRow {
        #[template_child]
        pub front_label: TemplateChild<gtk::Label>,
        #[template_child]
        pub detail_box: TemplateChild<gtk::Box>,
        #[template_child]
        pub back_label: TemplateChild<gtk::Label>,
        #[template_child]
        pub tag_pills: TemplateChild<TagPills>,
        #[template_child]
        pub suspended_icon: TemplateChild<gtk::Image>,
        /// A narrow row: fewer pills, so the back keeps some room.
        #[property(get, set)]
        pub compact: Cell<bool>,
        #[template_child]
        pub menu_button: TemplateChild<gtk::MenuButton>,
        #[template_child]
        pub card_menu: TemplateChild<gtk::gio::MenuModel>,
        /// The bound card, empty while unbound.
        pub card_id: RefCell<String>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for CardRow {
        const NAME: &'static str = "RappelCardRow";
        type Type = super::CardRow;
        type ParentType = gtk::Box;

        fn class_init(klass: &mut Self::Class) {
            TagPills::ensure_type();
            klass.bind_template();
            klass.install_action("row.edit", None, |row, _, _| {
                row.forward("deck.edit-card");
            });
            klass.install_action("row.info", None, |row, _, _| {
                row.forward("deck.card-info");
            });
            klass.install_action("row.suspend", None, |row, _, _| {
                row.forward("deck.suspend-card");
            });
            klass.install_action("row.unsuspend", None, |row, _, _| {
                row.forward("deck.unsuspend-card");
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

    #[glib::derived_properties]
    impl ObjectImpl for CardRow {
        fn constructed(&self) {
            self.parent_constructed();
            crate::ui::rows::menu_on_demand(&self.menu_button, &self.card_menu);
            let row = self.obj();
            row.connect_compact_notify(super::CardRow::update_pill_count);
            row.update_pill_count();
        }
    }
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
    /// Shows `item` in this row. Called on every bind of a recycled row,
    /// so it only sets texts and reuses the pills (`TagPills::set_tags`).
    pub fn bind(&self, item: &CardObject) {
        let imp = self.imp();
        let front = item.front_preview();
        imp.front_label.set_label(front);
        let back = item.back_preview();
        imp.back_label.set_label(back);
        let tags = item.tags();
        imp.tag_pills.set_tags(&tags);
        imp.tag_pills.set_visible(!tags.is_empty());
        // Without a back or tags, the row is its front alone.
        imp.detail_box
            .set_visible(!back.is_empty() || !tags.is_empty());
        rows::label_menu_button(&imp.menu_button, text::spoken_front(front));
        let suspended = item.card().suspended;
        imp.suspended_icon.set_visible(suspended);
        // The row offers the one of the two that changes something.
        self.action_set_enabled("row.suspend", !suspended);
        self.action_set_enabled("row.unsuspend", suspended);
        imp.card_id.replace(item.card().id.clone());
    }

    fn update_pill_count(&self) {
        let max = if self.compact() {
            COMPACT_ROW_PILLS
        } else {
            ROW_PILLS
        };
        self.imp().tag_pills.set_max_shown(max);
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

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    use rappel_core::domain::{Card, Tag, TaggedCard};

    use super::*;

    /// Card `i` of a deck: a Markdown front, a back and a few tags.
    fn card(i: usize) -> CardObject {
        let tags = (0..i % 6)
            .map(|t| Tag {
                id: format!("tag-{t}"),
                name: format!("tag number {t}"),
            })
            .collect();
        CardObject::new(TaggedCard {
            card: Card {
                id: format!("card-{i:05}"),
                deck_id: "deck".to_owned(),
                front_md: format!("**Card {i}**: what is $\\frac{{{i}}}{{2}}$?"),
                back_md: format!("Answer {i}, with `code` and a [link](x)"),
                created_at: 0,
                updated_at: 0,
                suspended: i.is_multiple_of(7),
            },
            tags,
        })
    }

    #[test]
    fn a_row_shows_the_card_and_reads_it_out() {
        crate::testing::with_widgets(|| {
            let row = CardRow::default();
            let item = card(4);
            row.bind(&item);
            let imp = row.imp();
            assert_eq!(imp.front_label.label(), "Card 4: what is \\frac{4}{2}?");
            assert_eq!(imp.back_label.label(), "Answer 4, with code and a link");
            assert_eq!(imp.tag_pills.tags().len(), 4);
            assert!(!imp.suspended_icon.is_visible());
            // The menu is built when it first opens, not with the row.
            assert!(imp.menu_button.popover().is_none());
            assert_eq!(
                item.row_label(),
                "Card 4: what is \\frac{4}{2}?, back: Answer 4, with code and a link, \
                 tags: tag number 0, tag number 1, tag number 2, tag number 3"
            );
            // A recycled row follows its new card.
            row.bind(&card(42));
            assert!(imp.suspended_icon.is_visible());
            assert!(!imp.tag_pills.is_visible());
        });
    }

    #[test]
    fn a_narrow_row_shows_fewer_pills() {
        crate::testing::with_widgets(|| {
            let row = CardRow::default();
            let pills = &row.imp().tag_pills;
            assert_eq!(pills.max_shown(), ROW_PILLS);
            row.set_compact(true);
            assert_eq!(pills.max_shown(), COMPACT_ROW_PILLS);
            row.set_compact(false);
            assert_eq!(pills.max_shown(), ROW_PILLS);
        });
    }

    #[test]
    fn the_menu_is_built_once_on_first_open() {
        crate::testing::with_widgets(|| {
            let window = gtk::Window::new();
            let row = CardRow::default();
            window.set_child(Some(&row));
            window.present();
            let button = &row.imp().menu_button;
            assert!(button.popover().is_none());
            button.popup();
            let first = button.popover().expect("opening the menu builds it");
            button.popdown();
            button.popup();
            assert_eq!(button.popover(), Some(first));
            button.popdown();
            window.destroy();
        });
    }

    /// Scrolling binds recycled rows over and over. What keeps it fast is
    /// checked as work, not time, so the test holds on a loaded machine
    /// (#67): after a first bind that makes the row's pills, binding it to
    /// 4,000 of 5,000 cards makes no widget (the same pill labels, no menu popover),
    /// and only the cards bound get their previews made. The CPU time of
    /// the thread (not the wall clock, which grows while the thread waits
    /// for a CPU) is printed with `RAPPEL_TIMINGS=1` and bounded only
    /// against a large regression: about 100 ms in a debug build (most of
    /// it making each card's two previews), 380 ms with three busy threads
    /// per CPU (shared caches slow the thread even while it runs).
    #[test]
    fn binding_a_row_to_thousands_of_cards_makes_no_widgets() {
        let cpu = crate::testing::with_widgets(|| {
            let items: Vec<CardObject> = (0..5_000).map(card).collect();
            let row = CardRow::default();
            let pills = &row.imp().tag_pills;
            // Card 5 has the most tags: every pill and the "+N" pill exist.
            row.bind(&items[5]);
            let made = pills.pill_widgets();
            let cpu_start = crate::testing::thread_cpu_time();
            let wall = Instant::now();
            for item in &items[..4_000] {
                row.bind(item);
                drop(item.row_label());
                row.unbind();
            }
            crate::testing::print_timing("CardRow::bind, 4,000 cards (wall)", wall.elapsed());
            let cpu = cpu_start
                .zip(crate::testing::thread_cpu_time())
                .map(|(start, end)| end.saturating_sub(start));
            assert_eq!(pills.pill_widgets(), made);
            assert!(row.imp().menu_button.popover().is_none());
            // Previews are made only for the cards a row showed.
            assert!(items[..4_000].iter().all(CardObject::has_previews));
            assert!(!items[4_000..].iter().any(CardObject::has_previews));
            cpu
        });
        if let Some(cpu) = cpu {
            crate::testing::print_timing("CardRow::bind, 4,000 cards (CPU)", cpu);
            // Over five times the time measured under load.
            assert!(cpu < Duration::from_secs(2), "{cpu:?}");
        }
    }
}
