//! `CardGroups`: the deck page's cards as its list shows them (ADR 0079),
//! grouped by the tags they have besides those every card has. A
//! `GListModel` and a `GtkSectionModel` over the page's own `gio::ListStore`
//! (the cards in creation order, which the page changes item by item):
//! each group is a section, which the list view gives a header
//! (`header-factory`, GTK 4.12), and the cards keep their order inside a
//! group. When grouping would not help ([`plan::grouping_helps`]) the cards
//! are in the store's order, as one section, and [`CardGroups::grouped`]
//! is false: the page then shows no headers.
//!
//! Every change of the store groups the cards again at once (one walk of
//! their tags, [`plan::plan`]), so the model never disagrees with the
//! store, and passes on one `items-changed` for the span that differs
//! ([`plan::changed_span`]): an edit that keeps the card's tags changes one
//! row, as before. Headers are told with `sections-changed` when a group
//! changed (its start, size or tags), which makes the list view make them
//! again (`gtk_list_item_manager_model_sections_changed_cb`,
//! gtk/gtklistitemmanager.c, source read for GTK 4.24); only headers on
//! screen exist, so that costs little.

pub mod plan;
pub mod text;

use std::cell::Ref;

use adw::prelude::*;
use adw::subclass::prelude::*;
use gtk::{gio, glib};
use rappel_core::domain::Tag;

use super::CardObject;
use plan::Group;

mod imp {
    use std::cell::{Cell, OnceCell, RefCell};

    use adw::prelude::*;
    use adw::subclass::prelude::*;
    use gtk::{gio, glib};

    use super::plan::{Plan, group_at};
    use crate::widgets::CardObject;

    #[derive(Debug, Default, glib::Properties)]
    #[properties(wrapper_type = super::CardGroups)]
    pub struct CardGroups {
        pub store: OnceCell<gio::ListStore>,
        /// The cards in the order shown.
        pub items: RefCell<Vec<CardObject>>,
        /// The groups and the shared tags.
        pub plan: RefCell<Plan>,
        /// The cards are shown in groups (more than one, and not too
        /// many); otherwise in the store's order without headers.
        #[property(get)]
        pub grouped: Cell<bool>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for CardGroups {
        const NAME: &'static str = "RappelCardGroups";
        type Type = super::CardGroups;
        type Interfaces = (gio::ListModel, gtk::SectionModel);
    }

    #[glib::derived_properties]
    impl ObjectImpl for CardGroups {}

    impl ListModelImpl for CardGroups {
        fn item_type(&self) -> glib::Type {
            CardObject::static_type()
        }

        fn n_items(&self) -> u32 {
            u32::try_from(self.items.borrow().len()).unwrap_or(u32::MAX)
        }

        fn item(&self, position: u32) -> Option<glib::Object> {
            let items = self.items.borrow();
            let item = items.get(usize::try_from(position).ok()?)?;
            Some(item.clone().upcast())
        }
    }

    impl SectionModelImpl for CardGroups {
        /// The group holding `position`, or the whole list when not
        /// grouped; past the end, from the end to `G_MAXUINT`, as
        /// `gtk_section_model_get_section` documents.
        fn section(&self, position: u32) -> (u32, u32) {
            let n = self.n_items();
            if position >= n {
                return (n, u32::MAX);
            }
            let plan = self.plan.borrow();
            match group_at(&plan.groups, position) {
                Some(group) => (group.start, group.start.saturating_add(group.len)),
                None => (0, n),
            }
        }
    }
}

glib::wrapper! {
    pub struct CardGroups(ObjectSubclass<imp::CardGroups>)
        @implements gio::ListModel, gtk::SectionModel;
}

impl CardGroups {
    /// The cards of `store` (`CardObject`s), grouped, following its
    /// changes. Made before anything else follows the store, so whatever
    /// the store's other handlers read of this model is already up to date
    /// (`GObject` signal handlers run in the order they were connected).
    pub fn new(store: &gio::ListStore) -> Self {
        let model: Self = glib::Object::new();
        crate::ui::init_once(&model.imp().store, store.clone());
        model.regroup();
        store.connect_items_changed(glib::clone!(
            #[weak]
            model,
            move |_, _, _, _| model.regroup()
        ));
        model
    }

    /// Groups the cards again after the store changed, and tells the list
    /// what changed.
    fn regroup(&self) {
        let imp = self.imp();
        let Some(store) = imp.store.get() else {
            return;
        };
        let cards: Vec<CardObject> = (0..store.n_items())
            .filter_map(|p| store.item(p).and_downcast::<CardObject>())
            .collect();
        let new_plan = {
            let lists: Vec<Ref<'_, Vec<Tag>>> = cards.iter().map(CardObject::tag_list).collect();
            plan::plan(lists.iter().map(|l| l.as_slice()))
        };
        let shown: Vec<CardObject> = if new_plan.grouped() {
            new_plan
                .order
                .iter()
                .filter_map(|&p| cards.get(usize::try_from(p).ok()?).cloned())
                .collect()
        } else {
            cards
        };
        let old = imp.items.replace(shown);
        let span = plan::changed_span(&old, &imp.items.borrow(), |a, b| a == b);
        drop(old);
        let was = imp.plan.replace(new_plan);
        let grouped = imp.plan.borrow().grouped();
        let groups_changed = !plan::same_groups(&was.groups, &imp.plan.borrow().groups);
        // `grouped` first, so every handler below reads the model whole.
        let grouped_changed = imp.grouped.replace(grouped) != grouped;
        if let Some((position, removed, added)) = span {
            let at = |n: usize| u32::try_from(n).unwrap_or(u32::MAX);
            self.items_changed(at(position), at(removed), at(added));
        }
        let n = self.n_items();
        if groups_changed && n > 0 {
            // Covers every position that was or is a section start
            // (`gtk_section_model_sections_changed` asks for that).
            self.sections_changed(0, n);
        }
        if grouped_changed {
            self.notify_grouped();
        }
    }

    /// The tags every card has (none for fewer than two cards).
    pub fn shared_tags(&self) -> Vec<Tag> {
        self.imp().plan.borrow().shared.clone()
    }

    /// Whether any tag is on every card.
    pub fn has_shared(&self) -> bool {
        !self.imp().plan.borrow().shared.is_empty()
    }

    /// The group starting at or holding `position`, while grouped.
    pub fn group_at(&self, position: u32) -> Option<Group> {
        plan::group_at(&self.imp().plan.borrow().groups, position).cloned()
    }

    /// The position at which card `id` is shown. A linear scan, only on
    /// a user action.
    pub fn position(&self, id: &str) -> Option<u32> {
        let items = self.imp().items.borrow();
        let i = items.iter().position(|c| c.card().id == id)?;
        u32::try_from(i).ok()
    }

    /// The plan, for the tests.
    #[cfg(test)]
    pub fn plan(&self) -> plan::Plan {
        self.imp().plan.borrow().clone()
    }
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::rc::Rc;
    use std::time::Duration;

    use rappel_core::domain::{Card, TaggedCard};

    use super::*;

    fn tag(name: &str) -> Tag {
        Tag {
            id: format!("id-{name}"),
            name: name.to_owned(),
        }
    }

    /// Card `i` with the shared tags "s1" and "s2", and "x" when `extra`.
    fn card(i: usize, extra: bool) -> CardObject {
        let mut tags = vec![tag("s1"), tag("s2")];
        if extra {
            tags.insert(0, tag("a-extra"));
        }
        CardObject::new(TaggedCard {
            card: Card {
                id: format!("card-{i:05}"),
                deck_id: "deck".to_owned(),
                front_md: format!("front {i}"),
                back_md: String::new(),
                created_at: 0,
                updated_at: 0,
                suspended: false,
            },
            tags,
        })
    }

    fn ids(model: &CardGroups) -> Vec<String> {
        (0..model.n_items())
            .filter_map(|p| model.item(p).and_downcast::<CardObject>())
            .map(|c| c.card().id.clone())
            .collect()
    }

    /// The changes the model passed on: (position, removed, added).
    fn record(model: &CardGroups) -> Rc<RefCell<Vec<(u32, u32, u32)>>> {
        let seen: Rc<RefCell<Vec<(u32, u32, u32)>>> = Rc::default();
        let log = Rc::clone(&seen);
        model.connect_items_changed(move |_, p, r, a| log.borrow_mut().push((p, r, a)));
        seen
    }

    #[test]
    fn the_model_groups_the_store_and_follows_it_item_by_item() {
        crate::testing::with_widgets(|| {
            let store = gio::ListStore::new::<CardObject>();
            let extra = [1, 3, 4];
            let items: Vec<CardObject> = (0..8).map(|i| card(i, extra.contains(&i))).collect();
            store.extend_from_slice(&items);
            let model = CardGroups::new(&store);
            assert!(model.grouped());
            assert_eq!(
                model
                    .shared_tags()
                    .iter()
                    .map(|t| t.name.clone())
                    .collect::<Vec<_>>(),
                ["s1", "s2"]
            );
            assert_eq!(
                ids(&model),
                [0, 2, 5, 6, 7, 1, 3, 4].map(|i| format!("card-{i:05}"))
            );
            assert_eq!(model.section(0), (0, 5));
            assert_eq!(model.section(4), (0, 5));
            assert_eq!(model.section(5), (5, 8));
            assert_eq!(model.section(8), (8, u32::MAX));
            assert_eq!(model.group_at(6).map(|g| g.tags.len()), Some(1));
            assert_eq!(model.position("card-00001"), Some(5));

            let changes = record(&model);
            let sections: Rc<RefCell<Vec<(u32, u32)>>> = Rc::default();
            let log = Rc::clone(&sections);
            model.connect_sections_changed(move |_, p, n| log.borrow_mut().push((p, n)));
            // An edit that keeps the tags: one row, no header.
            store.splice(3, 1, &[card(3, true)]);
            assert_eq!(*changes.borrow(), [(6, 1, 1)]);
            assert!(sections.borrow().is_empty());
            // A card gains the extra tag: it moves to the other group, and
            // the headers are told.
            changes.borrow_mut().clear();
            store.splice(2, 1, &[card(2, true)]);
            assert_eq!(
                ids(&model),
                [0, 5, 6, 7, 1, 2, 3, 4].map(|i| format!("card-{i:05}"))
            );
            assert_eq!(*changes.borrow(), [(1, 5, 5)]);
            assert_eq!(*sections.borrow(), [(0, 8)]);
            // A removal: one row.
            changes.borrow_mut().clear();
            store.remove(7);
            assert_eq!(*changes.borrow(), [(3, 1, 0)]);
            // Every card alike: no groups, the store's order.
            store.splice(0, store.n_items(), &[card(10, false), card(11, false)]);
            assert!(!model.grouped());
            assert_eq!(ids(&model), ["card-00010", "card-00011"]);
            assert_eq!(model.section(1), (0, 2));
            store.remove_all();
            assert_eq!(model.n_items(), 0);
            assert!(!model.has_shared());
        });
    }

    /// One change of a 10,000-card deck groups it again in time linear in
    /// its tags; measured as CPU time, bound far above it.
    #[test]
    fn a_change_of_ten_thousand_cards_is_grouped_quickly() {
        crate::testing::with_widgets(|| {
            let store = gio::ListStore::new::<CardObject>();
            let items: Vec<CardObject> = (0..10_000).map(|i| card(i, i % 3 == 0)).collect();
            store.extend_from_slice(&items);
            let model = CardGroups::new(&store);
            assert!(model.grouped());
            let changes = record(&model);
            let (cpu, ()) = crate::testing::cpu_time_of(|| {
                store.splice(5_000, 1, &[card(5_000, false)]);
            });
            assert_eq!(changes.borrow().len(), 1);
            crate::testing::print_timing("CardGroups: one change of 10,000 cards (CPU)", cpu);
            crate::testing::assert_cpu_within(cpu, Duration::from_millis(500), "regrouping");
        });
    }
}
