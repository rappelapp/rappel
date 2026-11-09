//! One row of a list of cards (ADR 0042, 0044, 0077, 0079): the front on
//! one line with the back dimmed under it if the card has one, each with
//! its formulas drawn (`render::MathLine`), the card's own tags as pills at
//! the row's end (not those every listed card shares, which the list says
//! once above its rows: the `shared-tags` property, `text::other_tags`; and
//! none on a deck page that groups its cards by their tags, `grouped`, whose
//! group headers name them), an icon when the card is suspended, a short
//! word for when the card is due (`text::due_word`), and a menu with Edit,
//! Card Information, Study Cards Tagged “…” for each of its tags (a submenu
//! when there are more than three, ADR 0069), Suspend or Unsuspend, Reset
//! Progress, Move to Deck… and Delete. The deck page and All Cards use the
//! same row, so both lists look and cost the same; on All Cards
//! ([`CardRow::for_browser`]) the second line starts with the deck. On
//! both, a check button shows while cards are being chosen
//! (`ui::selection`, ADR 0065), and a long press on a touch screen starts
//! choosing with the row's card.
//! Layout in `data/card_row.blp`; what a screen reader says in `text.rs`
//! (and `pages::browser_text` for All Cards).
//!
//! Rows are recycled by the `GtkListView`: [`CardRow::bind_card`] and
//! [`CardRow::bind_browse`] point a row at another card, reusing its pill
//! labels. The `compact` property (bound to the page's, a narrow window)
//! shows fewer pills; `selecting` shows the check button instead of the
//! menu; `shared-tags` (the ids of the tags every listed card has) leaves
//! those out of the pills; `now` and `next-day` (the page's clock and the
//! start of the next study day) set the due word. As in `DeckRow`, the
//! menu items activate the row's own `row.edit` / `row.info` /
//! `row.suspend` / `row.unsuspend` / `row.reset` / `row.move` /
//! `row.delete` actions, which forward to the page's `edit-card` /
//! `card-info` / `suspend-card` / `unsuspend-card` / `reset-card` /
//! `move-card` / `delete-card` actions (in the `deck.` or `browser.` group)
//! with the bound card's id, and a long press to `select-card`;
//! `row.study-tag`, whose target is a tag's id, forwards it to the page's
//! `study-tag`.

use adw::prelude::*;
use adw::subclass::prelude::*;
use gtk::{gio, glib};
use rappel_core::domain::Tag;

use super::tag_pills::text::{COMPACT_ROW_PILLS, ROW_PILLS};
use super::{BrowseItem, CardObject};
use crate::pages::browser_text;
use crate::ui::rows;
use crate::ui::selection::text::with_selected;
use rappel_core::content::Summary;
use text::TagMenu;

pub mod text;

mod imp {
    use std::cell::{Cell, OnceCell, RefCell};

    use adw::prelude::*;
    use adw::subclass::prelude::*;
    use gtk::{gio, glib};

    use crate::render::MathLine;
    use crate::widgets::TagPills;

    #[derive(Debug, Default, gtk::CompositeTemplate, glib::Properties)]
    #[properties(wrapper_type = super::CardRow)]
    #[template(resource = "/io/github/rappelapp/Rappel/card_row.ui")]
    pub struct CardRow {
        #[template_child]
        pub check: TemplateChild<gtk::CheckButton>,
        #[template_child]
        pub front_line: TemplateChild<MathLine>,
        #[template_child]
        pub detail_box: TemplateChild<gtk::Box>,
        #[template_child]
        pub back_line: TemplateChild<MathLine>,
        #[template_child]
        pub tag_pills: TemplateChild<TagPills>,
        #[template_child]
        pub caption_label: TemplateChild<gtk::Label>,
        #[template_child]
        pub suspended_icon: TemplateChild<gtk::Image>,
        #[template_child]
        pub due_label: TemplateChild<gtk::Label>,
        /// A narrow row: fewer pills, so the back keeps some room.
        #[property(get, set)]
        pub compact: Cell<bool>,
        /// Cards are being chosen: the check button shows, the menu does
        /// not.
        #[property(get, set)]
        pub selecting: Cell<bool>,
        /// The ids of the tags every card of the list has, which the page
        /// says once above it (`text::shared_tags`): the pills leave them
        /// out.
        #[property(get, set)]
        pub shared_tags: RefCell<Vec<String>>,
        /// The page groups its cards by their tags (ADR 0079): the group
        /// headers name the tags, so the row shows no pills.
        #[property(get, set)]
        pub grouped: Cell<bool>,
        /// The page's clock (UTC milliseconds) for the due word; 0 for
        /// the time of each bind.
        #[property(get, set)]
        pub now: Cell<i64>,
        /// When the next study day begins (UTC milliseconds), for
        /// "Tomorrow" and "In 3 days"; 0 for a day after `now`.
        #[property(get, set)]
        pub next_day: Cell<i64>,
        /// Where the bound card stands, for the due word.
        pub status: Cell<Option<rappel_core::storage::CardStatus>>,
        /// Another deck exists to move the card to (the page's, from
        /// `dialogs::move_card::offered`): Move to Deck… is enabled.
        #[property(get, set)]
        pub can_move: Cell<bool>,
        #[template_child]
        pub menu_button: TemplateChild<gtk::MenuButton>,
        #[template_child]
        pub card_menu: TemplateChild<gtk::gio::MenuModel>,
        /// The menu's section of Study Cards Tagged “…” entries, filled
        /// from the bound card's tags (`text::tag_menu`).
        pub tag_section: OnceCell<gio::Menu>,
        /// The tags `tag_section` was made for, so a rebind with the same
        /// tags (most cards of a deck have few, or none) leaves it alone.
        pub menu_tags: RefCell<Option<Vec<rappel_core::domain::Tag>>>,
        /// A row of All Cards: its actions are `browser.*`, not `deck.*`.
        pub browsing: Cell<bool>,
        /// The bound card, empty while unbound.
        pub card_id: RefCell<String>,
        /// The item shown (a `BrowseItem` or a `CardObject`), whose tags
        /// the pills show again when `shared-tags` changes.
        pub shown: RefCell<Option<glib::Object>>,
        /// The bound item (a `BrowseItem` or a `CardObject`), whose
        /// `selected` the row follows, and the list item whose accessible
        /// label says it.
        pub item: RefCell<Option<glib::Object>>,
        pub list_item: glib::WeakRef<gtk::ListItem>,
        pub selected_handler: RefCell<Option<glib::SignalHandlerId>>,
        /// How many times an item's `selected` reached the row, so the
        /// tests can tell that an old item's handler is gone.
        #[cfg(test)]
        pub selected_seen: Cell<usize>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for CardRow {
        const NAME: &'static str = "RappelCardRow";
        type Type = super::CardRow;
        type ParentType = gtk::Box;

        fn class_init(klass: &mut Self::Class) {
            TagPills::ensure_type();
            MathLine::ensure_type();
            klass.bind_template();
            klass.install_action("row.edit", None, |row, _, _| {
                row.forward("edit-card");
            });
            klass.install_action("row.info", None, |row, _, _| {
                row.forward("card-info");
            });
            klass.install_action("row.suspend", None, |row, _, _| {
                row.forward("suspend-card");
            });
            klass.install_action("row.unsuspend", None, |row, _, _| {
                row.forward("unsuspend-card");
            });
            klass.install_action("row.reset", None, |row, _, _| {
                row.forward("reset-card");
            });
            klass.install_action("row.move", None, |row, _, _| {
                row.forward("move-card");
            });
            klass.install_action("row.delete", None, |row, _, _| {
                row.forward("delete-card");
            });
            klass.install_action(
                "row.study-tag",
                Some(glib::VariantTy::STRING),
                |row, _, tag| {
                    if let Some(tag) = tag.and_then(glib::Variant::str) {
                        row.forward_tag(tag);
                    }
                },
            );
        }

        fn instance_init(obj: &glib::subclass::InitializingObject<Self>) {
            obj.init_template();
        }
    }

    #[glib::derived_properties]
    impl ObjectImpl for CardRow {
        fn constructed(&self) {
            self.parent_constructed();
            // After Edit and Card Information: a section of the card's own.
            let section = gio::Menu::new();
            if let Some(menu) = self.card_menu.downcast_ref::<gio::Menu>() {
                menu.insert_section(1, None, &section);
            }
            crate::ui::init_once(&self.tag_section, section);
            crate::ui::rows::menu_on_demand(&self.menu_button, &self.card_menu);
            let row = self.obj();
            row.connect_compact_notify(super::CardRow::update_pill_count);
            row.connect_compact_notify(super::CardRow::update_subtitle);
            row.update_pill_count();
            row.connect_selecting_notify(super::CardRow::update_selection);
            row.connect_shared_tags_notify(super::CardRow::update_pills);
            row.connect_grouped_notify(super::CardRow::update_pills);
            // The word and the row's label, which says it too.
            row.connect_now_notify(super::CardRow::update_due_and_label);
            row.connect_next_day_notify(super::CardRow::update_due_and_label);
            row.add_long_press();
            // `gtk_widget_action_set_enabled` (GtkWidget docs): a menu item
            // naming a disabled action is insensitive.
            row.connect_can_move_notify(|row| row.action_set_enabled("row.move", row.can_move()));
            row.action_set_enabled("row.move", row.can_move());
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
    /// A row of the deck page.
    fn default() -> Self {
        glib::Object::new()
    }
}

impl CardRow {
    /// A row of All Cards: its menu acts through `browser.*`, and its
    /// second line starts with the status and the deck.
    pub fn for_browser() -> Self {
        let row = Self::default();
        let imp = row.imp();
        imp.browsing.set(true);
        imp.caption_label.set_visible(true);
        row
    }

    /// A long press on a touch screen starts choosing cards with this one
    /// (a mouse has the Select button and Ctrl+A). Claiming the sequence
    /// keeps the release from activating the row (`GtkGesture` docs: a
    /// claimed sequence cancels the other gestures).
    fn add_long_press(&self) {
        let press = gtk::GestureLongPress::new();
        press.set_touch_only(true);
        press.connect_pressed(glib::clone!(
            #[weak(rename_to = row)]
            self,
            move |gesture, _, _| {
                gesture.set_state(gtk::EventSequenceState::Claimed);
                row.forward("select-card");
            }
        ));
        self.add_controller(press);
    }

    /// Shows a deck page's `item` in this row, and follows whether it is
    /// chosen; `list_item`'s accessible label is what a screen reader says
    /// for the row.
    pub fn bind_card(&self, item: &CardObject, list_item: &gtk::ListItem) {
        self.unbind();
        self.bind(item);
        self.follow(item.upcast_ref(), list_item);
    }

    /// Shows `item` in this row. Called on every bind of a recycled row,
    /// so it only sets texts and reuses the pills (`TagPills::set_tags`).
    pub fn bind(&self, item: &CardObject) {
        let card = item.card();
        self.show(&RowParts {
            front: item.front_summary(),
            back: item.back_summary(),
            pill_names: &item.tags(),
            tags: &item.tag_list(),
            suspended: card.suspended,
            is_new: item.is_new(),
            status: item.status(),
            id: &card.id,
        });
        self.imp().shown.replace(Some(item.clone().upcast()));
    }

    /// Shows an All Cards `item` in this row: the card, its deck and status,
    /// and whether it is chosen. `list_item`'s accessible label is what a
    /// screen reader says for the row.
    pub fn bind_browse(&self, item: &BrowseItem, list_item: &gtk::ListItem) {
        self.unbind();
        let imp = self.imp();
        {
            let row = item.row();
            let back = item.back_summary();
            self.show(&RowParts {
                front: item.front_summary(),
                back,
                pill_names: &item.tags(),
                tags: &row.tags,
                suspended: row.card.suspended,
                is_new: row.is_new,
                status: row.status,
                id: &row.card.id,
            });
        }
        imp.shown.replace(Some(item.clone().upcast()));
        self.update_subtitle();
        self.follow(item.upcast_ref(), list_item);
    }

    /// Follows `item`'s `selected` (a `BrowseItem`'s or a `CardObject`'s),
    /// and says the row in `list_item`'s accessible label.
    fn follow(&self, item: &glib::Object, list_item: &gtk::ListItem) {
        let imp = self.imp();
        imp.list_item.set(Some(list_item));
        imp.item.replace(Some(item.clone()));
        let handler = item.connect_notify_local(
            Some("selected"),
            glib::clone!(
                #[weak(rename_to = row)]
                self,
                move |_, _| {
                    #[cfg(test)]
                    row.imp()
                        .selected_seen
                        .set(row.imp().selected_seen.get() + 1);
                    row.update_selection();
                }
            ),
        );
        imp.selected_handler.replace(Some(handler));
        self.update_selection();
    }

    fn show(&self, parts: &RowParts<'_>) {
        let &RowParts {
            front,
            back,
            pill_names,
            tags,
            suspended,
            is_new,
            status,
            id,
        } = parts;
        let imp = self.imp();
        imp.front_line.set_summary(front);
        imp.back_line.set_summary(back);
        imp.back_line.set_visible(!back.text.is_empty());
        self.show_pills(pill_names, tags);
        self.set_tag_menu(tags);
        // Without a back (and, on the deck page, nothing else to say), the
        // line is left out.
        imp.detail_box
            .set_visible(!back.text.is_empty() || imp.browsing.get());
        rows::label_menu_button(&imp.menu_button, &text::spoken_front(&front.text));
        imp.suspended_icon.set_visible(suspended);
        // As the mockup's suspended rows: the front dimmed too, besides the
        // icon (the back is dimmed already).
        if suspended {
            imp.front_line.add_css_class("dimmed");
        } else {
            imp.front_line.remove_css_class("dimmed");
        }
        // Rows are recycled, so this is set on every bind. The `row.*`
        // actions are widget actions installed in `class_init`;
        // `gtk_widget_action_set_enabled` (GtkWidget docs) disables one, and
        // a menu item naming a disabled action is insensitive.
        for (action, enabled) in text::row_actions(suspended, is_new) {
            self.action_set_enabled(action, enabled);
        }
        imp.card_id.replace(id.to_owned());
        imp.status.set(Some(status));
        self.update_due();
    }

    /// The due word at the row's end, for the bound card at the page's
    /// clock (`text::due_word`), with the accent colour when the card is
    /// due now.
    fn update_due(&self) {
        let imp = self.imp();
        let Some(status) = imp.status.get() else {
            return;
        };
        let word = self.due_word(status);
        imp.due_label.set_label(&word.text);
        if word.accent {
            imp.due_label.add_css_class("accent");
        } else {
            imp.due_label.remove_css_class("accent");
        }
    }

    /// The due word and the accessible label, after the page's clock
    /// moved.
    fn update_due_and_label(&self) {
        self.update_due();
        self.update_selection();
    }

    /// The due word of a card in `status` at the page's clock.
    fn due_word(&self, status: rappel_core::storage::CardStatus) -> text::DueWord {
        let now = match self.now() {
            0 => rappel_core::time::now_ms(),
            now => now,
        };
        let next_day = match self.next_day() {
            day if day > now => day,
            _ => now.saturating_add(crate::ui::time::DAY_MS),
        };
        text::due_word(status, now, next_day)
    }

    /// The pills for a card with `names` and `tags`: the ones the list does
    /// not say above it (`text::other_tags`).
    fn show_pills(&self, names: &[String], tags: &[Tag]) {
        let imp = self.imp();
        let shared = imp.shared_tags.borrow();
        if self.grouped() {
            imp.tag_pills.set_visible(false);
            return;
        }
        let others = text::other_tags(names, tags, &shared);
        imp.tag_pills.set_tags(&others);
        imp.tag_pills.set_visible(!others.is_empty());
    }

    /// An All Cards row's second line: the deck, then the back, except on
    /// a narrow row, which has room for the deck only
    /// (`browser_text::subtitle`). Follows `compact`. When the card is
    /// due is the word at the row's end.
    fn update_subtitle(&self) {
        let imp = self.imp();
        let shown = imp.shown.borrow().clone();
        let Some(item) = shown.as_ref().and_then(|o| o.downcast_ref::<BrowseItem>()) else {
            return;
        };
        let with_back = !item.back_preview().is_empty() && !self.compact();
        imp.caption_label
            .set_label(&browser_text::subtitle(&item.row().deck_name, with_back));
        imp.back_line.set_visible(with_back);
    }

    /// The pills again, after `shared-tags` changed.
    fn update_pills(&self) {
        let shown = self.imp().shown.borrow().clone();
        if let Some(item) = shown.as_ref().and_then(|o| o.downcast_ref::<CardObject>()) {
            self.show_pills(&item.tags(), &item.tag_list());
        } else if let Some(item) = shown.as_ref().and_then(|o| o.downcast_ref::<BrowseItem>()) {
            self.show_pills(&item.tags(), &item.row().tags);
        }
    }

    /// Fills the row menu's tag section for `tags` ([`text::tag_menu`]),
    /// unless it was made for them already. Before the menu first opens the
    /// section is only a model: the popover is built from it then
    /// (`rows::menu_on_demand`), and follows its changes afterwards
    /// (`GtkPopoverMenu` builds its items from the model and tracks its
    /// `items-changed`, `GtkPopoverMenu` docs).
    fn set_tag_menu(&self, tags: &[Tag]) {
        let imp = self.imp();
        if imp.menu_tags.borrow().as_deref() == Some(tags) {
            return;
        }
        let Some(section) = imp.tag_section.get() else {
            return;
        };
        section.remove_all();
        let menu = text::tag_menu(tags);
        let item = |label: &str, tag_id: &str| {
            gio::MenuItem::new(
                Some(label),
                Some(&gio::Action::print_detailed_name(
                    "row.study-tag",
                    Some(&tag_id.to_variant()),
                )),
            )
        };
        match &menu {
            TagMenu::None => {}
            TagMenu::Items(items) => {
                for (label, tag_id) in items {
                    section.append_item(&item(label, tag_id));
                }
            }
            TagMenu::Submenu(label, items) => {
                let submenu = gio::Menu::new();
                for (name, tag_id) in items {
                    submenu.append_item(&item(name, tag_id));
                }
                section.append_submenu(Some(label), &submenu);
            }
        }
        imp.menu_tags.replace(Some(tags.to_vec()));
    }

    /// The check button, the menu and the accessible label of a row, after
    /// `selecting` or the item's `selected` changed.
    fn update_selection(&self) {
        let imp = self.imp();
        let selecting = self.selecting();
        imp.check.set_visible(selecting);
        imp.menu_button.set_visible(!selecting);
        let item = imp.item.borrow();
        let Some(item) = item.as_ref() else {
            return;
        };
        let selected = item.property::<bool>("selected");
        imp.check.set_active(selected);
        let Some(list_item) = imp.list_item.upgrade() else {
            return;
        };
        let chosen = selecting.then_some(selected);
        if let Some(item) = item.downcast_ref::<BrowseItem>() {
            let row = item.row();
            // At the page's clock, as the due word.
            let now = match self.now() {
                0 => rappel_core::time::now_ms(),
                now => now,
            };
            let status = browser_text::status_text(row.status, now);
            list_item.set_accessible_label(&browser_text::row_label(
                &text::row_label(
                    item.front_preview(),
                    item.back_preview(),
                    &item.tags(),
                    false,
                ),
                &row.deck_name,
                &status,
                chosen,
            ));
        } else if let Some(item) = item.downcast_ref::<CardObject>() {
            // Screen readers read the front, the back, the tags and when
            // the card is due, not "row" (`card_row::text::row_label`).
            let label = text::with_due(
                item.row_label(),
                &self.due_word(item.status()),
                item.card().suspended,
            );
            list_item.set_accessible_label(&with_selected(label, chosen));
        }
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
        let imp = self.imp();
        imp.card_id.borrow_mut().clear();
        imp.shown.replace(None);
        imp.status.set(None);
        if let (Some(item), Some(handler)) = (imp.item.take(), imp.selected_handler.take()) {
            item.disconnect(handler);
        }
        imp.list_item.set(None);
    }

    /// The row's position in its list, while bound through
    /// [`Self::bind_card`] or [`Self::bind_browse`].
    pub fn position(&self) -> Option<u32> {
        let item = self.imp().list_item.upgrade()?;
        let position = item.position();
        (position != gtk::INVALID_LIST_POSITION).then_some(position)
    }

    /// The bound card's id, empty while unbound.
    pub fn card_id(&self) -> String {
        self.imp().card_id.borrow().clone()
    }

    /// Activates the page's `study-tag` with `tag_id` (a menu entry of the
    /// bound card's tags).
    fn forward_tag(&self, tag_id: &str) {
        if self.card_id().is_empty() {
            return;
        }
        let group = if self.imp().browsing.get() {
            "browser"
        } else {
            "deck"
        };
        rows::forward_id(self, &format!("{group}.study-tag"), tag_id);
    }

    /// Activates the page's action `name` with the bound card's id.
    fn forward(&self, name: &str) {
        let id = self.card_id();
        if id.is_empty() {
            return;
        }
        let group = if self.imp().browsing.get() {
            "browser"
        } else {
            "deck"
        };
        rows::forward_id(self, &format!("{group}.{name}"), &id);
    }
}

/// What a row shows of one card, from a deck page's `CardObject` or an All
/// Cards `BrowseItem`.
struct RowParts<'a> {
    /// The front's one-line summary.
    front: &'a Summary,
    /// The back's one-line summary.
    back: &'a Summary,
    /// The tag names on one line each, as the item keeps them for the
    /// pills (made once per item, not per bind).
    pill_names: &'a [String],
    /// The tags with their ids, for the menu's Study Cards Tagged entries.
    tags: &'a [Tag],
    suspended: bool,
    is_new: bool,
    /// Where it stands, for the due word.
    status: rappel_core::storage::CardStatus,
    /// The card's id.
    id: &'a str,
}

/// Says on `page` which tags every item of `store` has (each a `T`, whose
/// tags `tags` reads; `text::shared_tags`): their ids in its
/// `shared-tags` property, which the rows follow, and their names in its
/// `shared-tag-names`, which the `CardListHeader` above the
/// list shows (its `tags` property). A property is set only when it
/// changes, so the rows are not redrawn for nothing.
pub fn update_shared_tags<T: IsA<glib::Object>>(
    page: &impl IsA<glib::Object>,
    store: &gio::ListStore,
    tags: for<'a> fn(&'a T) -> std::cell::Ref<'a, [Tag]>,
) {
    // One item at a time, its tags borrowed only while they are added,
    // stopping once no tag is shared.
    let mut acc = text::SharedTags::default();
    for position in 0..store.n_items() {
        let Some(item) = store.item(position).and_downcast::<T>() else {
            continue;
        };
        if !acc.add(&tags(&item)) {
            break;
        }
    }
    set_shared_tags(page, &acc.finish());
}

/// Says on `page` that every listed card has `shared`: their ids in its
/// `shared-tags` property, which the rows follow, and their names in its
/// `shared-tag-names`, which the `CardListHeader` above the list shows. A
/// property is set only when it changes, so the rows are not redrawn for
/// nothing.
pub fn set_shared_tags(page: &impl IsA<glib::Object>, shared: &[Tag]) {
    let ids: Vec<String> = shared.iter().map(|t| t.id.clone()).collect();
    let names: Vec<String> = shared
        .iter()
        .map(|t| crate::ui::text::one_line(&t.name))
        .collect();
    let page = page.upcast_ref::<glib::Object>();
    if page.property::<Vec<String>>("shared-tags") != ids {
        page.set_property("shared-tags", ids);
    }
    if page.property::<Vec<String>>("shared-tag-names") != names {
        page.set_property("shared-tag-names", names);
    }
}

/// Runs [`update_shared_tags`] once the main loop is idle, once for any
/// number of list changes before then: a list of 10,000 cards changes one
/// row at a time (an edit, a bulk action's rows) and must not be read
/// whole for each (ADR 0073's bound on a row change). The run is at
/// `G_PRIORITY_HIGH_IDLE`, before GTK lays out and draws the next frame
/// (`GTK_PRIORITY_RESIZE` and `GDK_PRIORITY_REDRAW` are lower, GTK docs),
/// so no frame shows the old tags.
#[derive(Debug, Default)]
pub struct SharedTagsQueue {
    pending: std::rc::Rc<std::cell::Cell<bool>>,
}

impl SharedTagsQueue {
    pub fn queue<T: IsA<glib::Object>>(
        &self,
        page: &impl IsA<glib::Object>,
        store: &gio::ListStore,
        tags: for<'a> fn(&'a T) -> std::cell::Ref<'a, [Tag]>,
    ) {
        if self.pending.replace(true) {
            return;
        }
        let pending = std::rc::Rc::clone(&self.pending);
        let page = page.upcast_ref::<glib::Object>().downgrade();
        let store = store.downgrade();
        glib::idle_add_local_full(glib::Priority::HIGH_IDLE, move || {
            pending.set(false);
            if let (Some(page), Some(store)) = (page.upgrade(), store.upgrade()) {
                update_shared_tags(&page, &store, tags);
            }
            glib::ControlFlow::Break
        });
    }
}

/// A list factory of card rows, for the deck page's list and All Cards':
/// `make` makes each row, which then follows `page`'s properties named in
/// `follows` (`compact`, `selecting`, `shared-tags`); `bind` shows the list item's item, a
/// `T`, in its row; and a row is unbound before it is recycled.
pub fn factory<T: IsA<glib::Object>>(
    page: &impl IsA<glib::Object>,
    make: fn() -> CardRow,
    follows: &'static [&'static str],
    bind: fn(&CardRow, &T, &gtk::ListItem),
) -> gtk::SignalListItemFactory {
    let factory = gtk::SignalListItemFactory::new();
    let page = page.upcast_ref::<glib::Object>().downgrade();
    factory.connect_setup(move |_, item| {
        let Some(item) = item.downcast_ref::<gtk::ListItem>() else {
            return;
        };
        let row = make();
        if let Some(page) = page.upgrade() {
            for &property in follows {
                page.bind_property(property, &row, property)
                    .sync_create()
                    .build();
            }
        }
        item.set_child(Some(&row));
    });
    factory.connect_bind(move |_, item| {
        let Some(item) = item.downcast_ref::<gtk::ListItem>() else {
            return;
        };
        if let (Some(shown), Some(row)) = (
            item.item().and_downcast::<T>(),
            item.child().and_downcast::<CardRow>(),
        ) {
            bind(&row, &shown, item);
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

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use rappel_core::domain::{Card, Tag, TaggedCard};
    use rappel_core::storage::{BrowseRow, CardStatus};

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

    /// Card `i` as All Cards lists it.
    fn browse_item(i: usize) -> BrowseItem {
        let item = card(i);
        BrowseItem::new(BrowseRow {
            card: item.card().clone(),
            deck_name: "Maths".to_owned(),
            tags: (0..i % 6)
                .map(|t| Tag {
                    id: format!("tag-{t}"),
                    name: format!("tag number {t}"),
                })
                .collect(),
            status: if i.is_multiple_of(7) {
                CardStatus::Suspended
            } else {
                CardStatus::New
            },
            is_new: true,
        })
    }

    #[test]
    fn a_row_shows_the_card_and_reads_it_out() {
        crate::testing::with_widgets(|| {
            let row = CardRow::default();
            let item = card(4);
            row.bind(&item);
            let imp = row.imp();
            // The formula is drawn (#128): a placeholder in the text.
            let front = imp.front_line.content();
            assert_eq!(front.text, "Card 4: what is \u{FFFC}?");
            assert_eq!(front.formulas, [(16, "\\frac{4}{2}".to_owned())]);
            assert_eq!(
                imp.back_line.content().text,
                "Answer 4, with code and a link"
            );
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
    fn an_all_cards_row_says_the_deck_and_follows_the_selection() {
        crate::testing::with_widgets(|| {
            let row = CardRow::for_browser();
            let list_item = glib::Object::new::<gtk::ListItem>();
            let item = browse_item(4);
            row.bind_browse(&item, &list_item);
            let imp = row.imp();
            assert_eq!(imp.caption_label.label(), "\u{2068}Maths\u{2069} ·");
            assert_eq!(imp.due_label.label(), "New");
            assert!(imp.back_line.is_visible());
            // A narrow row: the deck only.
            row.set_compact(true);
            assert_eq!(imp.caption_label.label(), "\u{2068}Maths\u{2069}");
            assert!(!imp.back_line.is_visible());
            row.set_compact(false);
            assert!(imp.caption_label.is_visible());
            assert!(!imp.check.is_visible() && imp.menu_button.is_visible());
            assert_eq!(imp.tag_pills.tags().len(), 4);
            assert_eq!(
                list_item.accessible_label(),
                ("Card 4: what is \\frac{4}{2}?, back: Answer 4, with code and a link, \
                     tags: tag number 0, tag number 1, tag number 2, tag number 3, \
                     deck: Maths, New")
            );
            // Choosing cards: the check button shows the item's state.
            row.set_selecting(true);
            assert!(imp.check.is_visible() && !imp.menu_button.is_visible());
            assert!(!imp.check.is_active());
            item.set_selected(true);
            assert!(imp.check.is_active());
            assert!(
                list_item
                    .accessible_label()
                    .ends_with("deck: Maths, New, selected")
            );
            // Recycled, it no longer follows the old item: the old item's
            // handler is gone, the new one's is there.
            let other = browse_item(7);
            row.bind_browse(&other, &list_item);
            assert!(!imp.check.is_active());
            let seen = imp.selected_seen.get();
            item.set_selected(false);
            assert_eq!(
                imp.selected_seen.get(),
                seen,
                "the old item reached the row"
            );
            other.set_selected(true);
            assert_eq!(imp.selected_seen.get(), seen + 1);
            assert!(imp.check.is_active());
            assert_eq!(imp.caption_label.label(), "\u{2068}Maths\u{2069} ·");
            assert_eq!(imp.due_label.label(), "Suspended");
            assert!(imp.suspended_icon.is_visible());
            // A deck page row has no deck and status.
            assert!(!CardRow::default().imp().caption_label.is_visible());
        });
    }

    /// The menu of a deck page row reaches `deck.*`, of an All Cards row
    /// `browser.*`, with the bound card's id.
    #[test]
    fn a_row_forwards_to_its_own_pages_actions() {
        crate::testing::with_widgets(|| {
            use std::cell::RefCell;
            use std::rc::Rc;

            let got: Rc<RefCell<Vec<String>>> = Rc::default();
            let group = |prefix: &'static str| {
                let actions = gtk::gio::SimpleActionGroup::new();
                let action =
                    gtk::gio::SimpleAction::new("edit-card", Some(glib::VariantTy::STRING));
                let got = got.clone();
                action.connect_activate(move |_, id| {
                    let id = id.and_then(glib::Variant::str).unwrap_or_default();
                    got.borrow_mut().push(format!("{prefix}.edit-card {id}"));
                });
                actions.add_action(&action);
                actions
            };
            let outer = gtk::Box::new(gtk::Orientation::Vertical, 0);
            outer.insert_action_group("deck", Some(&group("deck")));
            let inner = gtk::Box::new(gtk::Orientation::Vertical, 0);
            inner.insert_action_group("browser", Some(&group("browser")));
            outer.append(&inner);

            let deck_row = CardRow::default();
            inner.append(&deck_row);
            deck_row.bind(&card(3));
            deck_row.activate_action("row.edit", None).unwrap();

            let browser_row = CardRow::for_browser();
            inner.append(&browser_row);
            let list_item = glib::Object::new::<gtk::ListItem>();
            browser_row.bind_browse(&browse_item(4), &list_item);
            browser_row.activate_action("row.edit", None).unwrap();

            assert_eq!(
                *got.borrow(),
                ["deck.edit-card card-00003", "browser.edit-card card-00004"]
            );
        });
    }

    /// Move to Deck… sits above Delete in the card menu, is disabled until
    /// the page says another deck exists (`can-move`), and forwards to the
    /// page's `move-card` with the card's id.
    #[test]
    fn move_to_deck_follows_the_page_and_forwards_to_it() {
        crate::testing::with_widgets(|| {
            use std::cell::RefCell;
            use std::rc::Rc;

            let row = CardRow::default();
            let menu = row.imp().card_menu.get();
            let mut actions = Vec::new();
            for section in 0..menu.n_items() {
                let Some(items) = menu.item_link(section, "section") else {
                    continue;
                };
                for i in 0..items.n_items() {
                    actions.extend(
                        items
                            .item_attribute_value(i, "action", Some(glib::VariantTy::STRING))
                            .and_then(|v| v.get::<String>()),
                    );
                }
            }
            let at = |name: &str| actions.iter().position(|a| a == name).unwrap();
            assert_eq!(at("row.move") + 1, at("row.delete"), "{actions:?}");

            let got: Rc<RefCell<Vec<String>>> = Rc::default();
            let group = gtk::gio::SimpleActionGroup::new();
            let action = gtk::gio::SimpleAction::new("move-card", Some(glib::VariantTy::STRING));
            let seen = got.clone();
            action.connect_activate(move |_, id| {
                let id = id.and_then(glib::Variant::str).unwrap_or_default();
                seen.borrow_mut().push(id.to_owned());
            });
            group.add_action(&action);
            let outer = gtk::Box::new(gtk::Orientation::Vertical, 0);
            outer.insert_action_group("deck", Some(&group));
            outer.append(&row);
            row.bind(&card(3));

            // No other deck known: the item is insensitive and does nothing.
            assert!(!row.can_move());
            let _ = row.activate_action("row.move", None);
            assert!(got.borrow().is_empty());

            row.set_can_move(true);
            row.activate_action("row.move", None).unwrap();
            assert_eq!(*got.borrow(), ["card-00003"]);
        });
    }

    /// The `row.study-tag` entries of the menu's tag section: (label,
    /// target), and the submenu's label if they are in one.
    fn study_entries(row: &CardRow) -> (Vec<(String, String)>, Option<String>) {
        let section = row.imp().tag_section.get().unwrap();
        let read = |menu: &gtk::gio::MenuModel| -> Vec<(String, String)> {
            (0..menu.n_items())
                .map(|i| {
                    let label = menu
                        .item_attribute_value(i, "label", Some(glib::VariantTy::STRING))
                        .and_then(|v| v.get::<String>())
                        .unwrap();
                    let action = menu
                        .item_attribute_value(i, "action", Some(glib::VariantTy::STRING))
                        .and_then(|v| v.get::<String>())
                        .unwrap();
                    assert_eq!(action, "row.study-tag");
                    let target = menu
                        .item_attribute_value(i, "target", Some(glib::VariantTy::STRING))
                        .and_then(|v| v.get::<String>())
                        .unwrap();
                    (label, target)
                })
                .collect()
        };
        if section.n_items() == 1
            && let Some(sub) = section.item_link(0, "submenu")
        {
            let label = section
                .item_attribute_value(0, "label", Some(glib::VariantTy::STRING))
                .and_then(|v| v.get::<String>());
            return (read(&sub), label);
        }
        (read(section.upcast_ref()), None)
    }

    /// Each tag of the card has a Study Cards Tagged “…” entry, in a
    /// section after Edit and Card Information (a submenu past three
    /// tags), that follows a recycled row's card and forwards the tag's
    /// id to its page's `study-tag` (ADR 0069).
    #[test]
    fn the_menu_studies_the_cards_tags_and_forwards_to_the_page() {
        crate::testing::with_widgets(|| {
            use std::cell::RefCell;
            use std::rc::Rc;

            let row = CardRow::default();
            // The tag section is the menu's second.
            let menu = row.imp().card_menu.get();
            let second = menu.item_link(1, "section").unwrap();
            assert_eq!(
                second,
                row.imp()
                    .tag_section
                    .get()
                    .unwrap()
                    .clone()
                    .upcast::<gtk::gio::MenuModel>()
            );
            row.bind(&card(2));
            let label = |name: &str| format!("Study Cards Tagged “\u{2068}{name}\u{2069}”");
            assert_eq!(
                study_entries(&row),
                (
                    vec![
                        (label("tag number 0"), "tag-0".to_owned()),
                        (label("tag number 1"), "tag-1".to_owned())
                    ],
                    None
                )
            );
            // More than three tags: a submenu of their names.
            row.bind(&card(5));
            let (entries, submenu) = study_entries(&row);
            assert_eq!(submenu.as_deref(), Some("Study Cards _Tagged"));
            assert_eq!(entries.len(), 5);
            assert_eq!(
                entries[4],
                (
                    "\u{2068}tag number 4\u{2069}".to_owned(),
                    "tag-4".to_owned()
                )
            );
            // No tag, no entry.
            row.bind(&card(0));
            assert_eq!(study_entries(&row), (Vec::new(), None));

            let got: Rc<RefCell<Vec<String>>> = Rc::default();
            let group = |prefix: &'static str| {
                let actions = gtk::gio::SimpleActionGroup::new();
                let action =
                    gtk::gio::SimpleAction::new("study-tag", Some(glib::VariantTy::STRING));
                let got = got.clone();
                action.connect_activate(move |_, id| {
                    let id = id.and_then(glib::Variant::str).unwrap_or_default();
                    got.borrow_mut().push(format!("{prefix} {id}"));
                });
                actions.add_action(&action);
                actions
            };
            let outer = gtk::Box::new(gtk::Orientation::Vertical, 0);
            outer.insert_action_group("deck", Some(&group("deck")));
            outer.insert_action_group("browser", Some(&group("browser")));
            row.bind(&card(2));
            outer.append(&row);
            row.activate_action("row.study-tag", Some(&"tag-1".to_variant()))
                .unwrap();
            let browser_row = CardRow::for_browser();
            outer.append(&browser_row);
            browser_row.bind_browse(&browse_item(3), &glib::Object::new::<gtk::ListItem>());
            assert_eq!(study_entries(&browser_row).0.len(), 3);
            browser_row
                .activate_action("row.study-tag", Some(&"tag-2".to_variant()))
                .unwrap();
            assert_eq!(*got.borrow(), ["deck tag-1", "browser tag-2"]);
        });
    }

    /// A tag's name reaches the opened menu as it is: an underscore is not
    /// taken as a mnemonic, and markup is not parsed.
    #[test]
    fn a_tag_name_is_shown_as_it_is_in_the_menu() {
        crate::testing::with_widgets(|| {
            let window = gtk::Window::new();
            let row = CardRow::default();
            window.set_child(Some(&row));
            window.present();
            let item = CardObject::new(TaggedCard {
                card: card(1).card().clone(),
                tags: vec![Tag {
                    id: "t".to_owned(),
                    name: "my_tag <b>x</b>".to_owned(),
                }],
            });
            row.bind(&item);
            let button = &row.imp().menu_button;
            button.popup();
            let popover = button.popover().unwrap();
            let mut texts = Vec::new();
            texts.extend(crate::testing::label_texts(popover.upcast_ref()));
            button.popdown();
            window.destroy();
            assert!(
                texts.contains(&"Study Cards Tagged “\u{2068}my_tag <b>x</b>\u{2069}”".to_owned()),
                "{texts:?}"
            );
        });
    }

    /// The tags every listed card has are left out of the pills, and come
    /// back when the list no longer shares them; the accessible label still
    /// says every tag.
    #[test]
    fn a_row_leaves_out_the_shared_tags() {
        crate::testing::with_widgets(|| {
            let row = CardRow::default();
            let item = card(4);
            row.bind_card(&item, &glib::Object::new::<gtk::ListItem>());
            let pills = &row.imp().tag_pills;
            assert_eq!(pills.tags().len(), 4);
            row.set_shared_tags(vec!["tag-0".to_owned(), "tag-2".to_owned()]);
            assert_eq!(pills.tags(), ["tag number 1", "tag number 3"]);
            assert!(pills.is_visible());
            row.set_shared_tags((0..4).map(|t| format!("tag-{t}")).collect::<Vec<_>>());
            assert!(!pills.is_visible());
            assert!(item.row_label().contains("tag number 0"));
            row.set_shared_tags(Vec::<String>::new());
            assert_eq!(pills.tags().len(), 4);
            // A recycled row follows the list's shared tags for its new card.
            row.set_shared_tags(vec!["tag-0".to_owned()]);
            row.bind(&card(2));
            assert_eq!(pills.tags(), ["tag number 1"]);
            // An All Cards row too.
            let browser_row = CardRow::for_browser();
            browser_row.set_shared_tags(vec!["tag-1".to_owned()]);
            browser_row.bind_browse(&browse_item(3), &glib::Object::new::<gtk::ListItem>());
            assert_eq!(
                browser_row.imp().tag_pills.tags(),
                ["tag number 0", "tag number 2"]
            );
        });
    }

    /// A deck page row ends with its due word ("Today" in the accent
    /// colour), follows the page's clock, says it in its label, shows no
    /// pills while the page groups its cards, and has no second line
    /// without a back.
    #[test]
    fn a_row_says_when_its_card_is_due() {
        crate::testing::with_widgets(|| {
            use rappel_core::storage::CardStanding;
            const NOW: i64 = 1_000_000_000_000;
            const DAY: i64 = crate::ui::time::DAY_MS;
            let row = CardRow::default();
            row.set_now(NOW);
            row.set_next_day(NOW + DAY / 2);
            let list_item = glib::Object::new::<gtk::ListItem>();
            let item = card(4).with_standing(CardStanding {
                status: CardStatus::Due(NOW - DAY),
                is_new: false,
            });
            row.bind_card(&item, &list_item);
            let imp = row.imp();
            assert_eq!(imp.due_label.label(), "Today");
            assert!(imp.due_label.has_css_class("accent"));
            assert!(list_item.accessible_label().ends_with(", due today"));
            // The clock moves on: an hour before it, the card was not due.
            let later = card(5).with_standing(CardStanding {
                status: CardStatus::Scheduled(NOW + DAY),
                is_new: false,
            });
            row.bind_card(&later, &list_item);
            assert_eq!(imp.due_label.label(), "Tomorrow");
            assert!(!imp.due_label.has_css_class("accent"));
            row.set_now(NOW + DAY + 1);
            assert_eq!(imp.due_label.label(), "Today");
            assert!(list_item.accessible_label().ends_with(", due today"));
            // Grouped, no pills; the label still says every tag.
            assert!(imp.tag_pills.is_visible());
            row.set_grouped(true);
            assert!(!imp.tag_pills.is_visible());
            assert!(list_item.accessible_label().contains("tag number 4"));
            row.set_grouped(false);
            assert!(imp.tag_pills.is_visible());
            // No back: one line.
            let front_only = CardObject::new(rappel_core::domain::TaggedCard {
                card: Card {
                    back_md: String::new(),
                    ..card(1).card().clone()
                },
                tags: Vec::new(),
            });
            row.bind_card(&front_only, &list_item);
            assert!(!imp.detail_box.is_visible());
            row.bind_card(&item, &list_item);
            assert!(imp.detail_box.is_visible());
        });
    }

    #[test]
    fn a_suspended_card_has_its_front_dimmed() {
        crate::testing::with_widgets(|| {
            let row = CardRow::default();
            row.bind(&card(7));
            assert!(row.imp().front_line.has_css_class("dimmed"));
            row.bind(&card(1));
            assert!(!row.imp().front_line.has_css_class("dimmed"));
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
    /// against a large regression: about 100 ms in a debug build before
    /// the rows drew formulas (#128), about 380 ms since (every card here
    /// has a formula of its own, so each bind lays one out with `RaTeX`;
    /// ADR 0077), and 380 ms before that with three busy threads per CPU
    /// (shared caches slow the thread even while it runs).
    #[test]
    fn binding_a_row_to_thousands_of_cards_makes_no_widgets() {
        crate::testing::with_widgets(|| {
            let items: Vec<CardObject> = (0..5_000).map(card).collect();
            let row = CardRow::default();
            let pills = &row.imp().tag_pills;
            // Card 5 has the most tags: every pill and the "+N" pill exist.
            row.bind(&items[5]);
            let made = pills.pill_widgets();
            let (cpu, ()) = crate::testing::cpu_time_of(|| {
                for item in &items[..4_000] {
                    row.bind(item);
                    drop(item.row_label());
                    row.unbind();
                }
            });
            assert_eq!(pills.pill_widgets(), made);
            assert!(row.imp().menu_button.popover().is_none());
            // Previews are made only for the cards a row showed.
            assert!(items[..4_000].iter().all(CardObject::has_previews));
            assert!(!items[4_000..].iter().any(CardObject::has_previews));
            crate::testing::print_timing("CardRow::bind, 4,000 cards (CPU)", cpu);
            // Over five times the time measured under load; checked on this
            // thread, which did the work (ADR 0049).
            crate::testing::assert_cpu_within(cpu, Duration::from_secs(2), "CardRow::bind");
        });
    }

    /// As above, for the rows of All Cards (ADR 0044): binding one row to
    /// 4,000 of 5,000 cards makes no pill label and no popover, makes the
    /// previews of those cards only, and leaves no handler on the items it
    /// was bound to before.
    #[test]
    fn binding_an_all_cards_row_to_thousands_of_cards_makes_no_widgets() {
        crate::testing::with_widgets(|| {
            let items: Vec<BrowseItem> = (0..5_000).map(browse_item).collect();
            let list_item = glib::Object::new::<gtk::ListItem>();
            let row = CardRow::for_browser();
            row.set_selecting(true);
            let pills = &row.imp().tag_pills;
            row.bind_browse(&items[5], &list_item);
            let made = pills.pill_widgets();
            let (cpu, ()) = crate::testing::cpu_time_of(|| {
                for item in &items[..4_000] {
                    row.bind_browse(item, &list_item);
                    row.unbind();
                }
            });
            assert_eq!(pills.pill_widgets(), made);
            assert!(row.imp().menu_button.popover().is_none());
            assert!(items[..4_000].iter().all(BrowseItem::has_previews));
            assert!(!items[4_000..].iter().any(BrowseItem::has_previews));
            // Unbound, the row no longer follows any of them: no handler
            // is left on the items it was bound to.
            let seen = row.imp().selected_seen.get();
            for item in &items[..4_000] {
                item.set_selected(true);
            }
            assert_eq!(row.imp().selected_seen.get(), seen);
            crate::testing::print_timing("CardRow::bind_browse, 4,000 cards (CPU)", cpu);
            // The same bound as the deck page's rows, far above the time
            // measured.
            crate::testing::assert_cpu_within(cpu, Duration::from_secs(2), "CardRow::bind_browse");
        });
    }

    /// Card `i` with `n` tags of 60 bytes each, as an import may give.
    fn heavily_tagged(i: usize, n: usize) -> TaggedCard {
        let mut card = card(i).card().clone();
        card.id = format!("tagged-{i}");
        TaggedCard {
            card,
            tags: (0..n)
                .map(|t| Tag {
                    id: format!("tag-{t}"),
                    name: format!("{t:05} {}", "x".repeat(54)),
                })
                .collect(),
        }
    }

    /// A card with 5,000 tags: a row makes only the pills it shows and the
    /// "+N" pill, its tooltip and accessible label name 20 tags and count
    /// the rest, and binding stays cheap. Twenty such cards bound 200 times
    /// over took about 20 ms of CPU in a debug build.
    #[test]
    fn a_card_with_thousands_of_tags_binds_cheaply() {
        crate::testing::with_widgets(|| {
            let cards: Vec<TaggedCard> = (0..20).map(|i| heavily_tagged(i, 5_000)).collect();
            let items: Vec<CardObject> = cards.iter().cloned().map(CardObject::new).collect();
            let browse: Vec<BrowseItem> = cards
                .into_iter()
                .map(|tagged| {
                    BrowseItem::new(BrowseRow {
                        card: tagged.card,
                        deck_name: "Maths".to_owned(),
                        tags: tagged.tags,
                        status: CardStatus::New,
                        is_new: true,
                    })
                })
                .collect();
            let row = CardRow::default();
            let all_cards_row = CardRow::for_browser();
            let list_item = glib::Object::new::<gtk::ListItem>();
            row.bind(&items[0]);
            all_cards_row.bind_browse(&browse[0], &list_item);
            let (cpu, ()) = crate::testing::cpu_time_of(|| {
                for round in 0..10 {
                    for (item, browse_item) in items.iter().zip(&browse) {
                        row.bind(item);
                        drop(item.row_label());
                        row.unbind();
                        all_cards_row.bind_browse(browse_item, &list_item);
                        all_cards_row.unbind();
                    }
                    std::hint::black_box(round);
                }
            });
            for pills in [&row.imp().tag_pills, &all_cards_row.imp().tag_pills] {
                let made = pills.pill_widgets();
                assert_eq!(made.len(), ROW_PILLS as usize + 1);
                let overflow = made.last().map(|l| l.label().to_string());
                assert_eq!(overflow.as_deref(), Some("+4,997"));
                let tooltip = made
                    .last()
                    .and_then(gtk::prelude::WidgetExt::tooltip_text)
                    .unwrap_or_default();
                assert!(tooltip.ends_with(" and 4,977 more"), "{tooltip}");
                assert!(tooltip.len() < 2_000);
            }
            let said = items[0].row_label();
            assert!(said.contains(" and 4,980 more"), "{said}");
            assert!(said.len() < 2_000);
            crate::testing::print_timing("CardRow: 400 binds of 5,000 tags (CPU)", cpu);
            crate::testing::assert_cpu_within(
                cpu,
                Duration::from_millis(300),
                "binding 5,000 tags",
            );
        });
    }
}
