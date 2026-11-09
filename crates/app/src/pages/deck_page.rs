//! One deck's page: its cards in a `GtkListView`, or an empty state, with
//! add, edit, delete and reset progress (each undone from a toast). Layout in
//! `data/deck_page.blp` (ADR 0013). Selection mode, with its header, action
//! bar and bulk actions, is All Cards' (`ui::selection`, ADR 0065).
//!
//! The subtitle and the Study button follow what there is to study. A
//! one-shot timer re-checks it when the next card becomes due or the next
//! study day begins, so they change while the page is open.
//!
//! The cards are a `gio::ListStore` of [`CardObject`] in creation order.
//! Changes update it item by item instead of rebuilding it, so a deck of
//! thousands of cards keeps its scroll position and stays fast. The list
//! shows them through [`CardGroups`], grouped by the tags they have besides
//! the shared ones, each group under a header (ADR 0079): positions in the
//! store (to change it) and in the list (to focus a row) differ.

use std::cell::Cell;
use std::cmp::Ordering;
use std::collections::HashMap;
use std::rc::Rc;
use std::time::Duration;

use adw::prelude::*;
use adw::subclass::prelude::*;
use gtk::{gio, glib};
use rappel_core::Error;
use rappel_core::domain::{Card, Deck, TaggedCard};
use rappel_core::scheduling::{StudyInput, StudyScope};
use rappel_core::storage::{CardStanding, CardStatus};

use crate::dialogs::card_editor::{CardEditor, Saved};
use crate::dialogs::{card_info, merge_deck, move_card};
use crate::i18n::gettext;
use crate::pages::{ReviewPage, StartedFrom, StatsPage, Studied};
use crate::state::AppState;
use crate::ui::focus::{self, ShownFocus, index_after_removal};
use crate::ui::selection::{CardSelection, Parts, SelectionPage};
use crate::ui::text::due_new_line;
use crate::ui::text::with_error;
use crate::ui::toast::{info_toast, log_failure, show_error, show_toast_from, undo_toast};
use crate::ui::undo::{self, UndoFailure};
use crate::ui::{init_once, keys, nav};
use crate::widgets::{CardGroupHeader, CardGroups, CardObject, CardRow, card_row};
use crate::window::RappelWindow;

/// The empty state's icon, as in its layout, given back after a short
/// window took it off (`ui::breakpoint::compact_status_page`).
const EMPTY_ICON: &str = "document-new-symbolic";

mod imp {
    use std::cell::{Cell, OnceCell, RefCell};
    use std::rc::Rc;

    use adw::prelude::*;
    use adw::subclass::prelude::*;
    use gtk::{gio, glib};
    use rappel_core::domain::Deck;

    use crate::state::AppState;
    use crate::ui::breakpoint;
    use crate::ui::debounce::Debounce;
    use crate::ui::rows;
    use crate::ui::selection::CardSelection;
    use crate::widgets::{CardObject, SelectionBar, SelectionHeader};

    #[derive(Debug, Default, gtk::CompositeTemplate, glib::Properties)]
    #[properties(wrapper_type = super::DeckPage)]
    #[template(resource = "/io/github/rappelapp/Rappel/deck_page.ui")]
    pub struct DeckPage {
        #[template_child]
        pub empty_short: TemplateChild<adw::Breakpoint>,
        #[template_child]
        pub empty_status: TemplateChild<adw::StatusPage>,
        #[template_child]
        pub breakpoint: TemplateChild<adw::Breakpoint>,
        #[template_child]
        pub selection_narrow: TemplateChild<adw::Breakpoint>,
        #[template_child]
        pub header: TemplateChild<adw::HeaderBar>,
        #[template_child]
        pub select_button: TemplateChild<gtk::Button>,
        #[template_child]
        pub bar_select_button: TemplateChild<gtk::Button>,
        #[template_child]
        pub bar_counts: TemplateChild<gtk::Label>,
        #[template_child]
        pub selection_header: TemplateChild<SelectionHeader>,
        #[template_child]
        pub selection_bar: TemplateChild<SelectionBar>,
        #[template_child]
        pub window_title: TemplateChild<adw::WindowTitle>,
        #[template_child]
        pub content_stack: TemplateChild<gtk::Stack>,
        #[template_child]
        pub study_bar: TemplateChild<gtk::ActionBar>,
        #[template_child]
        pub header_study_button: TemplateChild<gtk::Button>,
        #[template_child]
        pub bar_study_button: TemplateChild<gtk::Button>,
        #[template_child]
        pub card_list: TemplateChild<gtk::ListView>,
        #[template_child]
        pub list_header: TemplateChild<crate::widgets::CardListHeader>,
        #[template_child]
        pub empty_add_button: TemplateChild<gtk::Button>,
        #[template_child]
        pub deck_menu: TemplateChild<gio::MenuModel>,
        #[template_child]
        pub deck_menu_button: TemplateChild<gtk::MenuButton>,
        /// The cards changed since `tag_section` was filled: it is filled
        /// again when the Deck Menu opens, not on every change.
        pub tag_menu_stale: Cell<bool>,
        /// The Deck Menu's section with the "Study Cards Tagged" submenu
        /// of the deck's tags (ADR 0069), empty while no card has a tag.
        pub tag_section: OnceCell<gio::Menu>,
        /// What `tag_section` shows, so an unchanged list is left alone.
        pub tag_menu: RefCell<Option<crate::widgets::card_row::text::DeckTagMenu>>,
        pub state: OnceCell<AppState>,
        pub deck: OnceCell<Deck>,
        /// The cards, oldest first.
        pub cards: OnceCell<gio::ListStore>,
        /// The cards as the list shows them, grouped (ADR 0079).
        pub groups: OnceCell<crate::widgets::CardGroups>,
        /// The list view's group headers, given to it while the cards are
        /// grouped.
        pub header_factory: OnceCell<gtk::SignalListItemFactory>,
        /// An editor is open; a second one (Ctrl+N, a click) is not opened.
        pub editing: Rc<Cell<bool>>,
        /// The card to focus when the editor closes: the one last saved.
        pub focus_after_edit: RefCell<Option<String>>,
        /// The pending re-check of what there is to study.
        pub study_timer: Debounce,
        /// A narrow window (the breakpoint applies): the rows show fewer
        /// tag pills.
        #[property(get, set)]
        pub compact: Cell<bool>,
        /// Another deck exists to move a card to: the rows' Move to
        /// Deck… is enabled (`move_card::offered`), and the action bar's
        /// Move….
        #[property(get, set)]
        pub can_move: Cell<bool>,
        /// Cards are being chosen; the rows follow it.
        #[property(get, set)]
        pub selecting: Cell<bool>,
        /// The ids of the tags every card of the deck has; the rows leave
        /// them out of their pills (ADR 0077).
        #[property(get, set)]
        pub shared_tags: RefCell<Vec<String>>,
        /// Their names, which the list's header says once.
        #[property(get, set)]
        pub shared_tag_names: RefCell<Vec<String>>,
        /// The cards are shown in groups by their tags (`CardGroups`): the
        /// rows show no pills.
        #[property(get, set)]
        pub grouped: Cell<bool>,
        /// The page's clock for the rows' due words (UTC milliseconds), set
        /// when the page opens or is shown and when the study timer runs.
        #[property(get, set)]
        pub now: Cell<i64>,
        /// When the next study day begins, for the due words.
        #[property(get, set)]
        pub next_day: Cell<i64>,
        /// The window is under 500 sp (`selection_narrow` applies): with
        /// `compact`, Select All and the action bar keep only their icons.
        pub selection_narrow_on: Cell<bool>,
        /// Selection mode: the chosen cards, the header, the action bar and
        /// the bulk actions (`ui::selection`).
        pub selection: OnceCell<CardSelection<super::DeckPage, CardObject>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for DeckPage {
        const NAME: &'static str = "RappelDeckPage";
        type Type = super::DeckPage;
        type ParentType = adw::NavigationPage;

        fn class_init(klass: &mut Self::Class) {
            SelectionHeader::ensure_type();
            SelectionBar::ensure_type();
            klass.bind_template();
            klass.install_action("deck.new-card", None, |page, _, _| {
                page.new_card();
            });
            klass.install_action("deck.study", None, |page, _, _| {
                page.study();
            });
            klass.install_action("deck.practice", None, |page, _, _| {
                page.practise();
            });
            klass.install_action("deck.statistics", None, |page, _, _| {
                page.show_statistics();
            });
            klass.install_action("deck.merge", None, |page, _, _| {
                page.merge();
            });
            klass.install_action("deck.archive", None, |page, _, _| {
                page.archive();
            });
            // Activated by the card rows' menus with the card id.
            rows::install_id_action(klass, "deck.edit-card", super::DeckPage::edit);
            rows::install_id_action(klass, "deck.delete-card", super::DeckPage::delete);
            rows::install_id_action(klass, "deck.move-card", super::DeckPage::move_card);
            rows::install_id_action(klass, "deck.suspend-card", |page, id| {
                page.suspend(id, true);
            });
            rows::install_id_action(klass, "deck.unsuspend-card", |page, id| {
                page.suspend(id, false);
            });
            rows::install_id_action(klass, "deck.reset-card", super::DeckPage::reset);
            rows::install_id_action(klass, "deck.card-info", super::DeckPage::card_info);
            // A card's menu, with a tag's id (ADR 0069).
            rows::install_id_action(klass, "deck.study-tag", super::DeckPage::study_tag);
            // A long press on a touch screen: start choosing with the card.
            rows::install_id_action(klass, "deck.select-card", |page, id| {
                if let Some(selection) = page.imp().selection.get() {
                    selection.press(id);
                }
            });
        }

        fn instance_init(obj: &glib::subclass::InitializingObject<Self>) {
            obj.init_template();
        }
    }

    #[glib::derived_properties]
    impl ObjectImpl for DeckPage {
        fn constructed(&self) {
            self.parent_constructed();
            // The empty state on a short window (ADR 0058).
            crate::ui::breakpoint::compact_status_page(
                &self.empty_short,
                &self.empty_status,
                super::EMPTY_ICON,
            );
            // After Statistics: the deck's tags to study (ADR 0069).
            let section = gio::Menu::new();
            if let Some(menu) = self.deck_menu.downcast_ref::<gio::Menu>() {
                menu.insert_section(1, None, &section);
            }
            crate::ui::init_once(&self.tag_section, section);
            let page = self.obj();
            // `GtkMenuButton:active` turns on as the menu opens; the
            // popover follows its model's changes (`GtkPopoverMenu`), so
            // filling the section then shows the tags of now.
            self.deck_menu_button.connect_active_notify(glib::clone!(
                #[weak]
                page,
                move |button| {
                    if button.is_active() {
                        page.update_tag_menu();
                    }
                }
            ));
            // Back from studying: what is left to study changed, and so
            // may which cards are new. The rows are refreshed before the
            // focus is set, so it lands on a row that stays.
            page.connect_shown(|page| {
                page.update_clock();
                page.update_study();
                page.refresh_standings();
                page.focus_when_shown();
            });
            page.connect_compact_notify(|page| {
                // The subtitle is shorter on a phone (`study_subtitle`).
                page.update_study();
                page.update_study_bar();
                page.update_selection_compact();
            });
            // As on All Cards (ADR 0036).
            breakpoint::follow(
                &self.breakpoint,
                glib::clone!(
                    #[weak]
                    page,
                    move |compact| page.set_compact(compact)
                ),
            );
            breakpoint::follow(
                &self.selection_narrow,
                glib::clone!(
                    #[weak]
                    page,
                    move |narrow| {
                        page.imp().selection_narrow_on.set(narrow);
                        page.update_selection_compact();
                    }
                ),
            );
        }
    }
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
    pub fn new(state: AppState, deck: Deck) -> rappel_core::Result<Self> {
        let page: Self = glib::Object::new();
        let imp = page.imp();
        let now = rappel_core::time::now_ms();
        let cards = state.list_tagged_cards(&deck.id)?;
        let standings = state.deck_standings(&deck.id, now)?;
        // Deck names are untrusted; NavigationPage:title is plain text.
        page.set_title(&deck.name);
        imp.window_title.set_title(&deck.name);

        let store = gio::ListStore::new::<CardObject>();
        store.extend_from_slice(&items_of(cards, &standings));
        // Before anything else follows the store, so the groups are up to
        // date in every handler below (`CardGroups::new`).
        let groups = CardGroups::new(&store);
        init_once(
            &imp.selection,
            CardSelection::new(
                &page,
                state.clone(),
                Parts {
                    own_header: imp.header.get().upcast(),
                    select_button: imp.select_button.get(),
                    narrow_select_button: Some(imp.bar_select_button.get()),
                    header: imp.selection_header.get(),
                    bar: imp.selection_bar.get(),
                    list: imp.card_list.get(),
                    items: groups.clone().upcast(),
                    own_deck: Some(deck.id.clone()),
                },
            ),
        );
        store.connect_items_changed(glib::clone!(
            #[weak]
            page,
            move |_, _, _, _| {
                page.update_empty_state();
                page.update_study();
                // Read the tags when the Deck Menu opens: a change here
                // can be one of thousands of cards (ADR 0069).
                page.imp().tag_menu_stale.set(true);
                page.update_shared_tags();
                // Select follows whether there is a card; an emptied list
                // ends selection mode.
                page.selection().list_changed();
            }
        ));
        imp.card_list
            .set_model(Some(&gtk::NoSelection::new(Some(groups.clone()))));
        imp.card_list.set_factory(Some(&row_factory(&page)));
        init_once(&imp.header_factory, header_factory(&groups));
        groups
            .bind_property("grouped", &page, "grouped")
            .sync_create()
            .build();
        page.connect_grouped_notify(DeckPage::update_headers);
        page.update_headers();
        page.bind_property("shared-tag-names", &*imp.list_header, "tags")
            .sync_create()
            .build();
        imp.card_list.connect_activate(glib::clone!(
            #[weak]
            page,
            move |_, position| {
                // In selection mode a click chooses the card.
                if page.selection().activate(position) {
                    return;
                }
                if let Some(item) = page.shown_item(position) {
                    page.open_editor(Some(&item.card()));
                }
            }
        ));
        // Move… follows whether there is another deck.
        page.connect_can_move_notify(|page| page.selection().update());
        page.add_escape();
        page.add_study_key();

        init_once(&imp.state, state);
        init_once(&imp.deck, deck);
        init_once(&imp.cards, store);
        init_once(&imp.groups, groups);
        page.update_clock();
        page.update_shared_tags();
        page.update_empty_state();
        page.update_study();
        imp.tag_menu_stale.set(true);
        page.update_can_move();
        page.selection().list_changed();
        Ok(page)
    }

    /// Selection mode. Set in `new`, before anything can ask for it.
    #[expect(
        clippy::expect_used,
        reason = "an invariant: `new` sets the cell before it returns the page"
    )]
    fn selection(&self) -> &CardSelection<Self, CardObject> {
        self.imp()
            .selection
            .get()
            .expect("`DeckPage::new` sets the selection before it returns the page")
    }

    /// Escape leaves selection mode. Otherwise it is not handled, so the
    /// navigation view goes back (the page can be popped again then).
    fn add_escape(&self) {
        let keys = gtk::EventControllerKey::new();
        keys.connect_key_pressed(glib::clone!(
            #[weak(rename_to = page)]
            self,
            #[upgrade_or]
            glib::Propagation::Proceed,
            move |_, key, _, _| {
                if key == gtk::gdk::Key::Escape {
                    page.escape()
                } else {
                    glib::Propagation::Proceed
                }
            }
        ));
        self.add_controller(keys);
    }

    /// S starts studying, as the Study button does (ADR 0081), and the
    /// button's tooltip says so. The controller is in the bubble phase (the
    /// default for a `GtkEventControllerKey`, gtkeventcontrollerkey.c): a
    /// text field or a button that wants the letter has had it first; the
    /// page also asks `study_key` whether a text field has the focus. While
    /// cards are being chosen, S does nothing (the Study button is hidden
    /// then); `deck.study` is disabled when there is nothing to study, and
    /// activating a disabled action does nothing (`gtk_widget_activate_action`).
    fn add_study_key(&self) {
        let keys = gtk::EventControllerKey::new();
        keys.connect_key_pressed(glib::clone!(
            #[weak(rename_to = page)]
            self,
            #[upgrade_or]
            glib::Propagation::Proceed,
            move |_, key, _, modifiers| {
                let typing = keys::text_has_focus(&page);
                if keys::study_key(key, modifiers, typing) == Some(keys::StudyKey::Start)
                    && !page.selecting()
                    && page.activate_action("deck.study", None).is_ok()
                {
                    glib::Propagation::Stop
                } else {
                    glib::Propagation::Proceed
                }
            }
        ));
        self.add_controller(keys);
        let imp = self.imp();
        if let Some(label) = keys::label(keys::START_STUDY_TRIGGER) {
            let tip = keys::tooltip(&gettext("Study"), &label);
            imp.header_study_button.set_tooltip_text(Some(&tip));
            imp.bar_study_button.set_tooltip_text(Some(&tip));
        }
    }

    /// What Escape does: leave selection mode, or nothing (All Cards'
    /// `escape_action`, with no search to clear).
    fn escape(&self) -> glib::Propagation {
        use crate::ui::selection::text::{EscapeAction, escape_action};
        match escape_action(self.selection().active(), false) {
            EscapeAction::LeaveSelection => {
                self.selection().set_active(false);
                glib::Propagation::Stop
            }
            EscapeAction::ClearSearch | EscapeAction::Nothing => glib::Propagation::Proceed,
        }
    }

    /// On a phone (under 500 sp) Select All and the action bar keep only
    /// their icons, as on All Cards. Under 400 sp the page's own
    /// breakpoint applies instead of `selection_narrow` (the last one that
    /// matches does), hence both.
    fn update_selection_compact(&self) {
        let imp = self.imp();
        let narrow = imp.selection_narrow_on.get() || self.compact();
        imp.selection_header.set_compact(narrow);
        imp.selection_bar.set_compact(narrow);
    }

    /// Fills the Deck Menu's "Study Cards Tagged" submenu with the tags of
    /// the deck's cards (`card_row::text::deck_tag_menu`), each studying
    /// its tag (`deck.study-tag`); past the cap, the last entry opens All
    /// Cards (`win.search`, filtered to this deck), where every tag is in
    /// the filter. Does nothing unless the cards changed since
    /// (`tag_menu_stale`): run when the Deck Menu opens.
    fn update_tag_menu(&self) {
        let imp = self.imp();
        if !imp.tag_menu_stale.replace(false) {
            return;
        }
        let (Some(section), Some(store)) = (imp.tag_section.get(), imp.cards.get()) else {
            return;
        };
        let items: Vec<CardObject> = (0..store.n_items())
            .filter_map(|p| store.item(p).and_downcast::<CardObject>())
            .collect();
        let lists: Vec<_> = items.iter().map(CardObject::tag_list).collect();
        let menu = card_row::text::deck_tag_menu(lists.iter().flat_map(|l| l.iter()));
        drop(lists);
        if *imp.tag_menu.borrow() == menu {
            return;
        }
        section.remove_all();
        if let Some(menu) = &menu {
            let submenu = gio::Menu::new();
            for (label, id) in &menu.items {
                submenu.append_item(&gio::MenuItem::new(
                    Some(label),
                    Some(&gio::Action::print_detailed_name(
                        "deck.study-tag",
                        Some(&id.to_variant()),
                    )),
                ));
            }
            if let Some(more) = &menu.more {
                submenu.append(Some(more), Some("win.search"));
            }
            section.append_submenu(Some(&menu.label), &submenu);
        }
        imp.tag_menu.replace(menu);
    }

    /// Says which tags every card of the deck has: once above the list,
    /// and left out of the rows' pills (ADR 0077). `CardGroups` read them
    /// with the groups, once per change of the list.
    fn update_shared_tags(&self) {
        if let Some(groups) = self.imp().groups.get() {
            card_row::set_shared_tags(self, &groups.shared_tags());
        }
    }

    /// Gives the list its group headers while the cards are grouped, and
    /// takes them away otherwise (`gtk_list_view_set_header_factory`: a
    /// list view without a header factory shows no headers, `GtkListView`
    /// docs).
    fn update_headers(&self) {
        let imp = self.imp();
        let factory = imp.header_factory.get().filter(|_| self.grouped());
        if imp.card_list.header_factory().as_ref() != factory.map(Cast::upcast_ref) {
            imp.card_list.set_header_factory(factory);
        }
    }

    /// Sets the rows' clock and the start of the next study day, from
    /// which their due words are said.
    fn update_clock(&self) {
        let imp = self.imp();
        let now = rappel_core::time::now_ms();
        if let Some(state) = imp.state.get() {
            match state.next_day_start(now) {
                Ok(day) => self.set_next_day(day),
                // The words count days from a day after now instead.
                Err(err) => tracing::warn!("cannot read when the study day begins: {err}"),
            }
        }
        self.set_now(now);
    }

    /// Enables the rows' Move to Deck… while another deck exists, and the
    /// Deck Menu's Merge Into… while there is a deck to merge into.
    fn update_can_move(&self) {
        let imp = self.imp();
        let (Some(state), Some(deck)) = (imp.state.get(), imp.deck.get()) else {
            return;
        };
        match state.decks() {
            Ok(decks) => {
                self.set_can_move(move_card::offered(&decks, Some(&deck.id)));
                self.action_set_enabled("deck.merge", merge_deck::offered(&decks, &deck.id));
            }
            // The entry keeps its last state; the move itself looks again.
            Err(err) => tracing::warn!("the decks could not be read for Move to Deck: {err}"),
        }
    }

    /// Makes each row follow where its card stands (its due word and its
    /// Reset Progress): a card whose standing changed gets a new item in
    /// the same place, which rebinds its row. One query for the deck, and
    /// a splice only for the cards that changed.
    pub fn refresh_standings(&self) {
        let imp = self.imp();
        let (Some(state), Some(deck), Some(store)) =
            (imp.state.get(), imp.deck.get(), imp.cards.get())
        else {
            return;
        };
        let standings = match state.deck_standings(&deck.id, rappel_core::time::now_ms()) {
            Ok(standings) => standings,
            Err(err) => {
                // The rows keep their last state; the core refuses a reset
                // of a new card either way.
                tracing::warn!("cannot read where the cards stand: {err}");
                return;
            }
        };
        // Every position must give its item, or the standings would no
        // longer line up with the positions spliced below.
        let Some(items) = (0..store.n_items())
            .map(|position| self.item(position))
            .collect::<Option<Vec<CardObject>>>()
        else {
            return;
        };
        let pairs: Vec<(CardStanding, CardStanding)> = items
            .iter()
            .map(|item| {
                let was = item.standing();
                (was, standings.get(&item.card().id).copied().unwrap_or(was))
            })
            .collect();
        let Some(range) = changed_range(&pairs) else {
            return;
        };
        // One splice for every row whose standing changed, so the list's
        // `items-changed` handlers (counts, selection, groups) run once,
        // not once per row (k rows cost k × O(n) before). A row whose
        // standing did not change goes back as the same item.
        let replacements: Vec<CardObject> = items[range.clone()]
            .iter()
            .zip(&pairs[range.clone()])
            .map(|(item, &(was, is))| {
                if was == is {
                    item.clone()
                } else {
                    item.copy_with(is)
                }
            })
            .collect();
        let (Ok(start), Ok(len)) = (u32::try_from(range.start), u32::try_from(range.len())) else {
            return;
        };
        store.splice(start, len, &replacements);
    }

    /// A list item for `tagged`, standing as the store says. For a card
    /// that comes from an edit or an undo, where one query is cheap.
    fn item_for(&self, tagged: TaggedCard) -> CardObject {
        let standing = match self
            .imp()
            .state
            .get()
            .map(|s| s.browse_row(&tagged.card.id, rappel_core::time::now_ms()))
        {
            Some(Ok(Some(row))) => CardStanding {
                status: row.status,
                is_new: row.is_new,
            },
            Some(Ok(None)) | None => unread_standing(&tagged.card),
            Some(Err(err)) => {
                // Reset stays offered; the core refuses it for a new card.
                tracing::warn!("cannot read the state of a card: {err}");
                unread_standing(&tagged.card)
            }
        };
        CardObject::new(tagged).with_standing(standing)
    }

    /// The id of this page's deck.
    pub fn deck_id(&self) -> Option<String> {
        self.imp().deck.get().map(|d| d.id.clone())
    }

    /// Recomputes what there is to study: the subtitle and whether Study
    /// is enabled.
    pub fn update_study(&self) {
        let imp = self.imp();
        let (Some(state), Some(deck)) = (imp.state.get(), imp.deck.get()) else {
            return;
        };
        let now = rappel_core::time::now_ms();
        let scope = StudyScope::deck(&deck.id);
        match state.study_input(&scope, now) {
            Ok(input) => {
                let compact = self.compact();
                imp.window_title
                    .set_subtitle(&study_subtitle(&input, compact));
                // The full line, where the header shortened it: in the
                // Study bar, which is there only on a phone, and as the
                // title's tooltip.
                imp.bar_counts.set_label(&study_subtitle(&input, false));
                imp.window_title
                    .set_tooltip_text(compact.then(|| study_subtitle(&input, false)).as_deref());
                self.action_set_enabled("deck.study", !input.is_empty());
                // Practice goes over every card, due or not (ADR 0070).
                let practisable = state.has_practisable(&scope).unwrap_or_else(|err| {
                    tracing::warn!("cannot read what to practise: {err}");
                    false
                });
                self.action_set_enabled("deck.practice", practisable);
                match (
                    state.next_study_change(&scope, now),
                    state.next_day_start(now),
                ) {
                    (Ok(next), Ok(day)) => self.arm_study_timer(timer_delay_ms(now, next, day)),
                    // The subtitle is right now; it only will not update by
                    // itself until the next change on this page.
                    (Err(err), _) | (_, Err(err)) => {
                        tracing::warn!("cannot read the next due time: {err}");
                    }
                }
            }
            Err(err) => {
                imp.window_title.set_subtitle("");
                imp.bar_counts.set_label("");
                self.action_set_enabled("deck.study", false);
                self.action_set_enabled("deck.practice", false);
                // Translators: a toast: what is before ":" is its title, the rest is behind Details. {error} is why, in English.
                self.error_toast(&with_error(
                    &gettext("What to study could not be read: {error}"),
                    &err,
                ));
            }
        }
    }

    /// Re-checks what there is to study after `delay_ms`, replacing any
    /// pending re-check. Holds the page weakly: a closed page ends it.
    fn arm_study_timer(&self, delay_ms: u64) {
        let page = self.downgrade();
        self.imp()
            .study_timer
            .restart(Duration::from_millis(delay_ms), move || {
                if let Some(page) = page.upgrade() {
                    // A card came due, or a new study day began: the rows'
                    // due words too ("In 10 min" becomes "Today").
                    page.update_clock();
                    page.refresh_standings();
                    page.update_study();
                }
            });
    }

    /// Pushes a review page for this deck.
    fn study(&self) {
        let imp = self.imp();
        let (Some(state), Some(deck)) = (imp.state.get(), imp.deck.get()) else {
            return;
        };
        let Some(view) = nav::navigation_view(self) else {
            return;
        };
        // Cards saved from the session's editor update this list too.
        let on_saved = glib::clone!(
            #[weak(rename_to = deck_page)]
            self,
            move |saved| {
                deck_page.show_saved(saved);
            }
        );
        let pushed = nav::push_unless_on_top(&view, || {
            let page = ReviewPage::new(state.clone(), deck.clone(), on_saved)?;
            self.action_set_enabled("deck.study", false);
            Ok::<_, rappel_core::Error>(page)
        });
        if let Err(err) = pushed {
            // Translators: a toast: what is before ":" is its title, the rest is behind Details. {error} is why, in English.
            self.error_toast(&with_error(
                &gettext("The session could not start: {error}"),
                &err,
            ));
        }
    }

    /// Practice from the Deck Menu: pushes a practice of this deck, which
    /// writes nothing (ADR 0070). Its Back and its summary's Back to Deck
    /// come back here.
    fn practise(&self) {
        let imp = self.imp();
        let (Some(state), Some(deck)) = (imp.state.get(), imp.deck.get()) else {
            return;
        };
        if self.selecting() {
            return;
        }
        let Some(view) = nav::navigation_view(self) else {
            return;
        };
        // Cards edited during the practice update this list too.
        let on_saved = glib::clone!(
            #[weak(rename_to = deck_page)]
            self,
            move |saved| {
                deck_page.show_saved(saved);
            }
        );
        let studied = Studied::Deck {
            deck: deck.clone(),
            from: StartedFrom::DeckPage,
        };
        if let Err(err) = nav::push_unless_on_top(&view, || {
            ReviewPage::practice(state.clone(), studied, on_saved)
        }) {
            // Translators: a toast: what is before ":" is its title, the rest is behind Details. {error} is why, in English.
            self.error_toast(&with_error(
                &gettext("The session could not start: {error}"),
                &err,
            ));
        }
    }

    /// Pushes a session over the cards carrying tag `tag_id` in every deck
    /// that is not archived (a card's menu, ADR 0069). Its Back, and its
    /// summary's Back to Deck, come back here; `shown` then reads the
    /// counts again, which the session changed.
    fn study_tag(&self, tag_id: &str) {
        let Some(state) = self.imp().state.get() else {
            return;
        };
        if self.selecting() {
            return;
        }
        // Cards of this deck saved from the session's editor update this
        // list (`show_saved` leaves out the other decks' cards).
        let on_saved = glib::clone!(
            #[weak(rename_to = deck_page)]
            self,
            move |saved| {
                deck_page.show_saved(saved);
            }
        );
        if let Err(err) =
            ReviewPage::push_for_tag(self, state, tag_id, StartedFrom::DeckPage, on_saved)
        {
            // Translators: a toast: what is before ":" is its title, the rest is behind Details. {error} is why, in English.
            self.error_toast(&with_error(
                &gettext("The session could not start: {error}"),
                &err,
            ));
        }
    }

    /// Pushes this deck's statistics.
    fn show_statistics(&self) {
        let imp = self.imp();
        let (Some(state), Some(deck)) = (imp.state.get(), imp.deck.get()) else {
            return;
        };
        let Some(view) = nav::navigation_view(self) else {
            return;
        };
        if let Err(err) = nav::push_unless_on_top(&view, || StatsPage::new(state, Some(deck))) {
            // Translators: a toast: what is before ":" is its title, the rest is behind Details. {error} is why, in English.
            self.error_toast(&with_error(
                &gettext("The statistics could not be read: {error}"),
                &err,
            ));
        }
    }

    /// Merge Into… from the Deck Menu: asks for the deck to merge this one
    /// into and merges at once. The deck is gone then, so the page goes
    /// back to the deck list, with the focus on the deck it went into; the
    /// toast's Undo brings it back there (#103, ADR 0066).
    fn merge(&self) {
        let imp = self.imp();
        let (Some(state), Some(deck)) = (imp.state.get().cloned(), imp.deck.get().cloned()) else {
            return;
        };
        glib::spawn_future_local(glib::clone!(
            #[weak(rename_to = page)]
            self,
            async move {
                // Nothing to put back on close: the page leaves.
                let Some(done) = merge_deck::run(&page, &state, &deck.id, || {}).await else {
                    // The last other deck may have gone since the menu was
                    // built.
                    page.update_can_move();
                    return;
                };
                let Some(window) = page.root().and_downcast::<RappelWindow>() else {
                    return;
                };
                window.leave_merged_deck(&page, &state, done);
            }
        ));
    }

    /// Archive from the Deck Menu: archives this deck at once. It has no
    /// page then, so the page goes back to the deck list, with the focus on
    /// the row now at its place; the toast's Undo brings it back there
    /// (ADR 0068).
    fn archive(&self) {
        let imp = self.imp();
        let (Some(state), Some(deck)) = (imp.state.get(), imp.deck.get()) else {
            return;
        };
        if let Err(err) = state.archive_deck(&deck.id) {
            self.error_toast(&crate::pages::archived_text::not_archived(&err));
            return;
        }
        let Some(window) = self.root().and_downcast::<RappelWindow>() else {
            return;
        };
        window.leave_archived_deck(self, state, &deck.id);
    }

    /// Shows the information of card `id`.
    fn card_info(&self, id: &str) {
        if let Some(state) = self.imp().state.get() {
            // AdwDialog gives the focus back to the widget that had it (the
            // card's row, or its menu button), so there is nothing to do
            // when it closes.
            card_info::open(self, state, id, || {});
        }
    }

    /// Opens the editor for a new card in this deck. Not while cards are
    /// being chosen (Ctrl+N): the header shows the selection then, and
    /// Cancel or Escape comes first.
    pub fn new_card(&self) {
        if !self.selecting() {
            self.open_editor(None);
        }
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
        CardEditor::open(
            self,
            &imp.editing,
            state.clone(),
            &deck.id,
            card,
            glib::clone!(
                #[weak(rename_to = page)]
                self,
                move |saved| page.saved(saved)
            ),
            glib::clone!(
                #[weak(rename_to = page)]
                self,
                move || {
                    if let Some(id) = page.imp().focus_after_edit.take() {
                        page.focus_card(&id);
                    }
                }
            ),
        );
    }

    /// Puts a card saved by this page's editor into the list and marks it
    /// to take the focus once the editor closes.
    fn saved(&self, saved: Saved) {
        if let Some(id) = self.show_saved(saved) {
            self.imp().focus_after_edit.replace(Some(id));
        }
    }

    /// Puts a saved card of this deck into the list, wherever the save came
    /// from (the review page's editor too). Returns its id; `None` for a
    /// card of another deck, which a tag's session edits too.
    fn show_saved(&self, saved: Saved) -> Option<String> {
        let imp = self.imp();
        let store = imp.cards.get()?;
        // A tag's session edits cards of other decks too (ADR 0069).
        let deck_id = match &saved {
            Saved::Added(tagged) | Saved::Edited(tagged) => &tagged.card.deck_id,
        };
        if imp.deck.get().is_none_or(|deck| &deck.id != deck_id) {
            return None;
        }
        let tagged = match saved {
            Saved::Added(tagged) => {
                // A card just added has no reviews.
                let standing = CardStanding {
                    status: CardStatus::New,
                    is_new: true,
                };
                insert_in_order(
                    store,
                    &CardObject::new(tagged.clone()).with_standing(standing),
                );
                tagged
            }
            Saved::Edited(tagged) => {
                match self.position(&tagged.card.id) {
                    Some(position) => {
                        store.splice(position, 1, &[self.item_for(tagged.clone())]);
                    }
                    None => {
                        insert_in_order(store, &self.item_for(tagged.clone()));
                    }
                }
                tagged
            }
        };
        Some(tagged.card.id)
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
            // Translators: a toast: what is before ":" is its title, the rest is behind Details. {error} is why, in English.
            self.error_toast(&with_error(
                &gettext("The card could not be deleted: {error}"),
                &err,
            ));
            return;
        }
        let shown = self.shown_position(id);
        store.remove(position);
        self.focus_or_empty_state(self.after_removal(shown));

        // Undo must work after going back to the deck list too, when this
        // page may be gone: the closure keeps the state, not the page.
        let id = id.to_owned();
        let state = state.clone();
        let page = self.downgrade();
        let window = self
            .root()
            .and_downcast::<RappelWindow>()
            .map(|w| w.downgrade());
        // The front is not in the title: it can be long or many lines.
        let toast = undo_toast(&gettext("Card deleted"), move || {
            undo_card_delete(&state, &id, &page, window.as_ref());
        });
        show_toast_from(self, toast);
    }

    /// Move to Deck…: asks for a deck other than this one, moves the card
    /// there at once, and the card leaves the list; the toast's Undo moves
    /// it back (#102).
    fn move_card(&self, id: &str) {
        let imp = self.imp();
        let (Some(state), Some(deck)) = (imp.state.get().cloned(), imp.deck.get().cloned()) else {
            return;
        };
        let id = id.to_owned();
        glib::spawn_future_local(glib::clone!(
            #[weak(rename_to = page)]
            self,
            async move {
                let decks = match state.decks() {
                    Ok(decks) => decks,
                    Err(err) => {
                        // Translators: a toast: what is before ":" is its title, the rest is behind Details. {error} is why, in English.
                        page.error_toast(&with_error(
                            &gettext("The decks could not be read: {error}"),
                            &err,
                        ));
                        return;
                    }
                };
                let targets: Vec<Deck> = move_card::targets(&decks, &deck.id)
                    .into_iter()
                    .cloned()
                    .collect();
                if targets.is_empty() {
                    // The last other deck went since the menu was built.
                    page.update_can_move();
                    show_toast_from(&page, info_toast(&move_card::no_other_deck_text()));
                    return;
                }
                // The dialog gives the focus back to the widget that had it
                // (a row's menu button, now bound to another card) when it
                // closes, after the answer: the focus is set again then.
                let focus: Rc<Cell<Option<MovedFocus>>> = Rc::default();
                let closed = glib::clone!(
                    #[weak]
                    page,
                    #[strong]
                    focus,
                    move || {
                        if let Some(focus) = focus.take() {
                            page.focus_or_empty_state(focus.position());
                        }
                    }
                );
                if let Some(target) = move_card::choose_deck(&page, targets, closed).await {
                    focus.set(page.finish_move(&state, &id, &target));
                }
            }
        ));
    }

    /// Moves card `id` to `target` and takes its row out of the list.
    /// Returns where the focus went: the row now at the card's place, or
    /// `None` for the empty state; `None` if the card did not move.
    fn finish_move(&self, state: &AppState, id: &str, target: &Deck) -> Option<MovedFocus> {
        let store = self.imp().cards.get()?;
        let moved = match state.move_cards(&[id.to_owned()], &target.id) {
            Ok(moved) if !moved.is_empty() => moved,
            // Already in that deck (moved elsewhere meanwhile), or gone: the
            // list is old.
            Ok(_) | Err(Error::CardNotFound(_) | Error::DeckNotFound(_)) => {
                self.reload();
                show_toast_from(self, info_toast(&crate::ui::selection::text::stale_text()));
                return None;
            }
            Err(err) => {
                // Translators: a toast: what is before ":" is its title, the rest is behind Details. {error} is why, in English.
                self.error_toast(&with_error(
                    &gettext("The card could not be moved: {error}"),
                    &err,
                ));
                return None;
            }
        };
        let focus = self.position(id).map(|position| {
            let shown = self.shown_position(id);
            store.remove(position);
            let next = MovedFocus::after_removal(self.after_removal(shown));
            self.focus_or_empty_state(next.position());
            next
        });
        let window = self.root().and_downcast::<RappelWindow>();
        counts_changed(self, window.as_ref());

        // As for a delete: Undo must work from another page.
        let (id, state) = (id.to_owned(), state.clone());
        let page = self.downgrade();
        let window = window.map(|w| w.downgrade());
        let toast = move_card::moved_toast(&state.clone(), moved, move |result| {
            undo_card_move(&state, &id, result, &page, window.as_ref());
        });
        show_toast_from(self, toast);
        focus
    }

    /// Makes the card new again at once, keeping its history; the toast's
    /// Undo cancels the reset (ADR 0028, 0030).
    fn reset(&self, id: &str) {
        let Some(state) = self.imp().state.get() else {
            return;
        };
        let entry = match state.reset_card(id, rappel_core::time::now_ms()) {
            Ok(entry) => entry,
            Err(err) => {
                match reset_notice(&err) {
                    Notice::Info(text) => {
                        // The row was stale: bring it up to date.
                        self.refresh_standings();
                        self.focus_card(id);
                        show_toast_from(self, info_toast(&text));
                    }
                    Notice::Error(text) => self.error_toast(&text),
                }
                return;
            }
        };
        let window = self.root().and_downcast::<RappelWindow>();
        counts_changed(self, window.as_ref());
        // The row was replaced (the card is new now): keep the focus in
        // the list, on the same card.
        self.focus_card(id);

        // As for a delete: Undo must work from another page, so the closure
        // keeps the state and weak references, not the page.
        let (card_id, entry_id) = (id.to_owned(), entry.id);
        let state = state.clone();
        let page = self.downgrade();
        let window = window.map(|w| w.downgrade());
        let toast = undo_toast(&gettext("Progress reset"), move || {
            undo_reset(&state, &card_id, &entry_id, &page, window.as_ref());
        });
        show_toast_from(self, toast);
    }

    /// Suspends or unsuspends the card at once; the toast's Undo puts the
    /// flag back (ADR 0035). A suspended card leaves what there is to study.
    fn suspend(&self, id: &str, suspended: bool) {
        let Some(state) = self.imp().state.get() else {
            return;
        };
        let changed = match state.set_suspended(&[id.to_owned()], suspended) {
            Ok(changed) => changed,
            Err(err) => {
                // Translators: a toast: what is before ":" is its title, the rest is behind Details. {error} is why, in English.
                self.error_toast(&with_error(
                    &gettext("The card could not be changed: {error}"),
                    &err,
                ));
                return;
            }
        };
        if changed.is_empty() {
            return;
        }
        self.refresh_card(id);
        let window = self.root().and_downcast::<RappelWindow>();
        counts_changed(self, window.as_ref());

        // As for a delete: Undo must work from another page.
        let (id, state) = (id.to_owned(), state.clone());
        let page = self.downgrade();
        let window = window.map(|w| w.downgrade());
        let title = if suspended {
            gettext("Card suspended")
        } else {
            gettext("Card unsuspended")
        };
        let toast = undo_toast(&title, move || {
            undo_suspend(&state, &id, suspended, &page, window.as_ref());
        });
        show_toast_from(self, toast);
    }

    /// Reads card `id` again into its row, after its flag changed.
    fn refresh_card(&self, id: &str) {
        let imp = self.imp();
        let (Some(state), Some(store)) = (imp.state.get(), imp.cards.get()) else {
            return;
        };
        let Some(position) = self.position(id) else {
            return;
        };
        match (state.card(id), state.card_tags(id)) {
            (Ok(card), Ok(tags)) => {
                store.splice(position, 1, &[self.item_for(TaggedCard { card, tags })]);
            }
            (Err(err), _) | (_, Err(err)) => {
                // Translators: a toast: what is before ":" is its title, the rest is behind Details. {error} is why, in English.
                self.error_toast(&with_error(
                    &gettext("The card could not be read: {error}"),
                    &err,
                ));
            }
        }
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
        if self.position(&card.id).is_none() {
            // A deleted card keeps its tags (ADR 0029).
            let tags = imp
                .state
                .get()
                .and_then(|state| match state.card_tags(&card.id) {
                    Ok(tags) => Some(tags),
                    Err(err) => {
                        tracing::warn!("a restored card's tags could not be read: {err}");
                        None
                    }
                })
                .unwrap_or_default();
            let tagged = TaggedCard {
                card: card.clone(),
                tags,
            };
            insert_in_order(store, &self.item_for(tagged));
        }
        self.focus_card(&card.id);
    }

    /// Reads the deck's cards again, e.g. after an import added some. The
    /// subtitle and Study follow through `items-changed`.
    pub fn reload(&self) {
        let imp = self.imp();
        let (Some(state), Some(deck), Some(store)) =
            (imp.state.get(), imp.deck.get(), imp.cards.get())
        else {
            return;
        };
        self.update_can_move();
        match state.list_tagged_cards(&deck.id).and_then(|cards| {
            let now = rappel_core::time::now_ms();
            Ok((cards, state.deck_standings(&deck.id, now)?))
        }) {
            Ok((cards, standings)) => {
                // The cards chosen stay chosen if they are still here
                // (`ui::selection` follows the list's changes).
                store.splice(0, store.n_items(), &items_of(cards, &standings));
            }
            // Translators: a toast: what is before ":" is its title, the rest is behind Details. {error} is why, in English.
            Err(err) => self.error_toast(&with_error(
                &gettext("The cards could not be read: {error}"),
                &err,
            )),
        }
    }

    /// Focuses the first card (or Add Card) unless the focus is already
    /// on this page (see [`focus::when_shown`]). On a push the focus is
    /// set here, in `shown`, never before: a grab before or during the push
    /// is lost (`ui::nav`). At startup the page is pushed without animation
    /// before the window is shown, and `shown` is emitted during that push:
    /// with animations off `switch_page` skips the transition, whose `done`
    /// handler emits it (`adw_animation_skip`, `transition_done_cb`,
    /// source read for libadwaita 1.8 and 1.9). The window then keeps this focus widget
    /// (`gtk_window_show` moves the focus only when it has none,
    /// gtk/gtkwindow.c, source read for GTK 4.22). Observed with
    /// libadwaita 1.10.0 and GTK 4.24.1, where the window test runs.
    fn focus_when_shown(&self) {
        let inside = self
            .root()
            .and_then(|root| root.focus())
            .is_some_and(|focus| focus.is_ancestor(self));
        let rows = self.imp().cards.get().map_or(0, gio::ListStore::n_items);
        match focus::when_shown(inside, rows) {
            ShownFocus::Keep => {}
            ShownFocus::FirstRow => {
                // `scroll_to` with FOCUS only moves the list's focus item
                // while the focus is outside it; the grab then focuses that
                // item's row, making it if needed (`gtk_list_base_scroll_to`,
                // `gtk_list_base_grab_focus`, gtk/gtklistbase.c, source read
                // for GTK 4.20 and 4.22; observed with GTK 4.24.1).
                self.focus_first_row_when_laid_out();
            }
            ShownFocus::EmptyState => self.focus_or_empty_state(None),
        }
    }

    /// Focuses the first row, and keeps its group's header in sight above
    /// it once the list is laid out. Focusing a row scrolls it into view
    /// (`gtk_list_base_set_focus_child` → `gtk_list_base_scroll_to_item`,
    /// gtk/gtklistbase.c, source read for GTK 4.24). Before the list is
    /// laid out, its rows have no place yet and `gtk_list_view_get_allocation`
    /// puts the row at 0, so the row is anchored at the top of the view and
    /// the header above it is scrolled away (ADR 0077 saw it under Xvfb,
    /// every time the page opened). The focus is given at once anyway: at
    /// startup the page is pushed before the window is shown, and the
    /// window keeps the focus widget it has then (`gtk_window_show`,
    /// gtk/gtkwindow.c). The first layout is then the sign to put the view
    /// back at its top, not a number of frames: the list view gives its
    /// rows their places, then configures its scroll adjustment
    /// (`gtk_list_view_size_allocate` → `gtk_list_base_allocate` →
    /// `gtk_list_base_update_adjustments`, gtk/gtklistview.c), whose
    /// `changed` signal with a page size says it is done. Once that
    /// allocation is over (an idle callback: moving the anchor inside the
    /// list's own allocation would queue another one), and only while the
    /// first card still has the focus, `ui::focus::focus_row` sets the view
    /// to its very top, where the row is in view and stays. A list already
    /// laid out (a page shown again) needs no wait.
    fn focus_first_row_when_laid_out(&self) {
        let list = self.imp().card_list.get();
        let Some(adjustment) = list.vadjustment() else {
            return;
        };
        // `focus_row` sets the view at its very top first. That is needed
        // even with the row in view: a list view's first anchor is its
        // first item, not the header above it (`gtk_list_base_set_model`
        // anchors position 0 at the top; headers are tiles of their own,
        // gtk/gtklistitemmanager.c), so the first layout alone leaves the
        // view scrolled by the header's height (seen under Xvfb: 40 px
        // before any focus). The grab then focuses the row, making it if
        // needed (`gtk_list_base_grab_focus`, gtk/gtklistbase.c, source
        // read for GTK 4.20 to 4.24).
        focus::focus_row(&list, 0);
        list.grab_focus();
        if adjustment.page_size() > 0.0 {
            return;
        }
        let handler: Rc<std::cell::RefCell<Option<glib::SignalHandlerId>>> = Rc::default();
        let once = Rc::clone(&handler);
        let page = self.downgrade();
        let id = adjustment.connect_changed(move |adjustment| {
            if adjustment.page_size() <= 0.0 {
                return;
            }
            if let Some(id) = once.borrow_mut().take() {
                adjustment.disconnect(id);
            }
            let page = page.clone();
            glib::idle_add_local_once(move || {
                if let Some(page) = page.upgrade()
                    && page.first_card_focused()
                {
                    focus::focus_row(&page.imp().card_list, 0);
                }
            });
        });
        handler.replace(Some(id));
    }

    /// Whether the keyboard focus is on the row of the first card shown.
    fn first_card_focused(&self) -> bool {
        let list = &self.imp().card_list;
        let Some(item) = list.focus_child() else {
            return false;
        };
        let focus = list.root().and_then(|root| root.focus());
        let inside = focus.is_some_and(|w| w == item || w.is_ancestor(&item));
        let row = item.first_child().and_downcast::<CardRow>();
        let first = self.shown_item(0);
        inside
            && matches!((row, first), (Some(row), Some(first)) if row.card_id() == first.card().id)
    }

    /// Focuses the card at `position`, or with `None` the empty state's
    /// Add Card button.
    fn focus_or_empty_state(&self, position: Option<u32>) {
        match position {
            Some(position) => self.focus(position),
            None => {
                self.imp().empty_add_button.grab_focus();
            }
        }
    }

    /// Scrolls to the card at `position` and gives it the keyboard focus.
    fn focus(&self, position: u32) {
        focus::focus_row(&self.imp().card_list, position);
    }

    fn update_empty_state(&self) {
        let imp = self.imp();
        let empty = imp.cards.get().is_none_or(|s| s.n_items() == 0);
        imp.content_stack
            .set_visible_child_name(if empty { "empty" } else { "list" });
        self.update_study_bar();
    }

    /// Reveals the bottom Study bar only on a narrow window over a deck
    /// with cards (see [`study_bar_shown`]), and not while cards are being
    /// chosen, when the action bar is there instead.
    fn update_study_bar(&self) {
        let imp = self.imp();
        let empty = imp.cards.get().is_none_or(|s| s.n_items() == 0);
        imp.study_bar
            .set_revealed(study_bar_shown(self.compact(), empty) && !self.selecting());
    }

    /// The card at `position` of the store (creation order).
    fn item(&self, position: u32) -> Option<CardObject> {
        self.imp().cards.get()?.item(position).and_downcast()
    }

    /// The card at `position` of the list shown (grouped).
    fn shown_item(&self, position: u32) -> Option<CardObject> {
        self.imp().groups.get()?.item(position).and_downcast()
    }

    /// Where card `id` is in the store (see [`position_in`]), to change it.
    fn position(&self, id: &str) -> Option<u32> {
        position_in(self.imp().cards.get()?, id)
    }

    /// Where card `id` is in the list shown, to focus its row.
    fn shown_position(&self, id: &str) -> Option<u32> {
        self.imp().groups.get()?.position(id)
    }

    /// Scrolls to card `id` and gives its row the keyboard focus.
    fn focus_card(&self, id: &str) {
        if let Some(position) = self.shown_position(id) {
            self.focus(position);
        }
    }

    /// The row to focus once the card shown at `shown` has left the list
    /// (`index_after_removal`, in the list's positions), or `None` for the
    /// empty state.
    fn after_removal(&self, shown: Option<u32>) -> Option<u32> {
        let n = self.imp().groups.get().map_or(0, ListModelExt::n_items);
        index_after_removal(shown?, n)
    }

    fn error_toast(&self, message: &str) {
        show_error(self, "a card operation", message);
    }
}

/// Where card `id` is in `store`. A linear scan, only on a user action:
/// about a millisecond for 5,000 cards in a debug build (issue #42; see
/// `finding_the_last_of_5000_cards_is_fast`).
fn position_in(store: &gio::ListStore, id: &str) -> Option<u32> {
    (0..store.n_items()).find(|&position| {
        store
            .item(position)
            .and_downcast::<CardObject>()
            .is_some_and(|c| c.card().id == id)
    })
}

/// Updates the counts after a reset or its undo: the deck's subtitle and
/// Study button, and the deck list. Through `window` when there is one, since
/// the deck may have been reopened in a new page.
fn counts_changed(page: &DeckPage, window: Option<&RappelWindow>) {
    if let Some(window) = window {
        window.study_changed();
    } else {
        page.update_study();
        page.refresh_standings();
    }
}

/// What the user is told when a reset fails.
#[derive(Debug, PartialEq, Eq)]
enum Notice {
    /// Nothing is wrong and nothing was written: a short message.
    Info(String),
    /// Something failed: an error toast.
    Error(String),
}

/// A card that is already new has no progress to reset (the row was stale,
/// e.g. after an import): that is information, not an error.
fn reset_notice(err: &Error) -> Notice {
    match err {
        Error::CardAlreadyNew(_) => Notice::Info(crate::ui::selection::text::no_progress_text(1)),
        // Translators: a toast: what is before ":" is its title, the rest is behind Details. {error} is why, in English.
        err => Notice::Error(with_error(
            &gettext("The progress could not be reset: {error}"),
            &err,
        )),
    }
}

/// The positions from the first to the last whose value changed, of the
/// `(was, is)` pairs in list order; `None` when none changed. Pure.
fn changed_range<T: PartialEq>(flags: &[(T, T)]) -> Option<std::ops::Range<usize>> {
    let first = flags.iter().position(|(was, is)| was != is)?;
    let last = flags.iter().rposition(|(was, is)| was != is)?;
    Some(first..last + 1)
}

/// The list items for `cards`, `standings` saying where each stands.
fn items_of(cards: Vec<TaggedCard>, standings: &HashMap<String, CardStanding>) -> Vec<CardObject> {
    cards
        .into_iter()
        .map(|tagged| {
            let standing = standings
                .get(&tagged.card.id)
                .copied()
                .unwrap_or_else(|| unread_standing(&tagged.card));
            CardObject::new(tagged).with_standing(standing)
        })
        .collect()
}

/// Where a card stands when that could not be read: suspended or not as
/// the card says, otherwise new, with Reset Progress offered (the core
/// refuses it for a new card).
fn unread_standing(card: &Card) -> CardStanding {
    CardStanding {
        status: if card.suspended {
            CardStatus::Suspended
        } else {
            CardStatus::New
        },
        is_new: false,
    }
}

/// The Undo of a reset: cancels reset entry `entry_id` of card `card_id`.
/// If the card was studied since, the entry is no longer its latest and
/// nothing changes but a short message.
fn undo_reset(
    state: &AppState,
    card_id: &str,
    entry_id: &str,
    page: &glib::WeakRef<DeckPage>,
    window: Option<&glib::WeakRef<RappelWindow>>,
) {
    let window = window.and_then(glib::WeakRef::upgrade);
    match state.undo(card_id, entry_id, rappel_core::time::now_ms()) {
        Ok(_) => {
            if let Some(page) = page.upgrade() {
                counts_changed(&page, window.as_ref());
            } else if let Some(window) = &window {
                window.study_changed();
            }
        }
        Err(err) => {
            let message = match undo::failure(&err) {
                UndoFailure::Stale => {
                    // Not an error: the card moved on, and nothing was written.
                    if let Some(window) = &window {
                        show_toast_from(
                            window,
                            info_toast(&gettext("The reset can no longer be undone")),
                        );
                    }
                    return;
                }
                // Translators: a toast: what is before ":" is its title, the rest is behind Details. {error} is why, in English.
                UndoFailure::Failed => {
                    with_error(&gettext("The reset could not be undone: {error}"), &err)
                }
            };
            if let Some(window) = window {
                show_error(&window, "a card operation", &message);
            } else {
                log_failure("a card operation", &message);
            }
        }
    }
}

/// The Undo of a suspend (`suspended` is what the action did): puts the flag
/// of card `id` back, on `page` if it is still there, and updates the counts
/// through `window`.
fn undo_suspend(
    state: &AppState,
    id: &str,
    suspended: bool,
    page: &glib::WeakRef<DeckPage>,
    window: Option<&glib::WeakRef<RappelWindow>>,
) {
    let window = window.and_then(glib::WeakRef::upgrade);
    match state.restore_suspended(&[id.to_owned()], !suspended) {
        Ok(_) => {
            if let Some(page) = page.upgrade() {
                page.refresh_card(id);
                counts_changed(&page, window.as_ref());
            } else if let Some(window) = &window {
                window.study_changed();
            }
        }
        Err(err) => {
            // Translators: a toast: what is before ":" is its title, the rest is behind Details. {error} is why, in English.
            let message = with_error(&gettext("The change could not be undone: {error}"), &err);
            if let Some(window) = window {
                show_error(&window, "a card operation", &message);
            } else {
                log_failure("a card operation", &message);
            }
        }
    }
}

/// The Undo of a card delete: brings card `id` back, on `page` if it is
/// still there and through `window` (the deck may have been reopened in a
/// new page, or left). Errors go to the window's toasts.
fn undo_card_delete(
    state: &AppState,
    id: &str,
    page: &glib::WeakRef<DeckPage>,
    window: Option<&glib::WeakRef<RappelWindow>>,
) {
    let window = window.and_then(glib::WeakRef::upgrade);
    match state.undelete_card(id) {
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
            // Translators: a toast: what is before ":" is its title, the rest is behind Details. {error} is why, in English.
            let message = with_error(&gettext("The card could not be restored: {error}"), &err);
            if let Some(window) = window {
                show_error(&window, "a card operation", &message);
            } else {
                log_failure("a card operation", &message);
            }
        }
    }
}

/// Where the focus goes after a card moved out of the list.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MovedFocus {
    /// The row now at this position.
    Row(u32),
    /// No row is left: the empty state's Add Card button.
    EmptyState,
}

impl MovedFocus {
    /// From `index_after_removal`.
    fn after_removal(next: Option<u32>) -> Self {
        next.map_or(Self::EmptyState, Self::Row)
    }

    /// As `focus_or_empty_state` takes it.
    fn position(self) -> Option<u32> {
        match self {
            Self::Row(position) => Some(position),
            Self::EmptyState => None,
        }
    }
}

/// The Undo of a card move: the card is back in its deck, on `page` if it
/// is still there and through `window` (the deck may have been reopened in
/// a new page, or left). `result` is how many cards went back.
fn undo_card_move(
    state: &AppState,
    id: &str,
    result: rappel_core::Result<usize>,
    page: &glib::WeakRef<DeckPage>,
    window: Option<&glib::WeakRef<RappelWindow>>,
) {
    let window = window.and_then(glib::WeakRef::upgrade);
    match result {
        Ok(0) => {
            // Moved again, deleted, or its first deck is gone since.
            let text = move_card::not_moved_back_text();
            match window {
                Some(window) => show_toast_from(&window, info_toast(&text)),
                None => log_failure("a card operation", &text),
            }
        }
        Ok(_) => match state.card(id) {
            Ok(card) => {
                if let Some(page) = page.upgrade() {
                    page.restored(&card);
                }
                if let Some(window) = window {
                    window.card_restored(&card);
                }
            }
            Err(err) => undo_move_failed(&err, window.as_ref()),
        },
        Err(err) => undo_move_failed(&err, window.as_ref()),
    }
}

/// An error toast for an Undo of a move that failed.
fn undo_move_failed(err: &Error, window: Option<&RappelWindow>) {
    // Translators: a toast: what is before ":" is its title, the rest is behind Details. {error} is why, in English.
    let message = with_error(&gettext("The card could not be moved back: {error}"), err);
    if let Some(window) = window {
        show_error(window, "a card operation", &message);
    } else {
        log_failure("a card operation", &message);
    }
}

/// The deck page's subtitle: "1,234 due · 20 new", or "Nothing to study"
/// (the same counts as the deck list's rows, `ui::text::due_new_line`).
/// `compact` (a phone, where the header also holds the window buttons and
/// French "5 à revoir · 20 nouvelles" is cut) keeps one count: the due
/// cards if there are any, else the new ones; the page's tooltip has the
/// full line (ADR 0071).
fn study_subtitle(input: &StudyInput, compact: bool) -> String {
    let due = input.reviews.len() + input.steps.len();
    let new = input.new.len();
    let line = if compact && due > 0 {
        due_new_line(due, 0)
    } else {
        due_new_line(due, new)
    };
    line.unwrap_or_else(|| gettext("Nothing to study"))
}

/// Whether the bottom Study bar shows: on a narrow window (`compact`, where
/// Study leaves the header), unless the deck is empty. An empty deck shows
/// its status page with Add Card alone; a disabled Study pill under it
/// would only be noise. On a wide window the header's Study stays, disabled,
/// so the header does not change shape when the first card is added.
fn study_bar_shown(compact: bool, empty: bool) -> bool {
    compact && !empty
}

/// The longest single wait before re-checking what there is to study. A
/// later change is reached in steps, so a changed clock or time zone is
/// noticed within this long.
const MAX_STUDY_WAIT_MS: u64 = crate::ui::time::HOUR_MS.unsigned_abs();

/// How long to wait from `now` until the earlier of `next_change` (the next
/// card that becomes due, if any) and `next_day` (the next study day, which
/// renews the new cards), capped at [`MAX_STUDY_WAIT_MS`]. All times are
/// UTC milliseconds; a time already past gives 0.
fn timer_delay_ms(now: i64, next_change: Option<i64>, next_day: i64) -> u64 {
    let at = next_change.map_or(next_day, |change| change.min(next_day));
    u64::try_from(at.saturating_sub(now))
        .unwrap_or(0)
        .min(MAX_STUDY_WAIT_MS)
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

/// Builds a [`CardRow`] per visible row, following the page's `compact`,
/// `can-move`, `selecting`, `shared-tags`, `grouped`, `now` and
/// `next-day`, and points it at its card (whose accessible label it sets,
/// `CardRow::bind_card`).
fn row_factory(page: &DeckPage) -> gtk::SignalListItemFactory {
    card_row::factory(
        page,
        CardRow::default,
        &[
            "compact",
            "can-move",
            "selecting",
            "shared-tags",
            "grouped",
            "now",
            "next-day",
        ],
        CardRow::bind_card,
    )
}

/// The list view's group headers (ADR 0079): a [`CardGroupHeader`] per
/// group on screen, showing the group that starts at the header's `start`
/// in `groups`. The list view makes a header again when its group changed
/// (`CardGroups` emits `sections-changed`), so a bind is all it needs.
fn header_factory(groups: &CardGroups) -> gtk::SignalListItemFactory {
    let factory = gtk::SignalListItemFactory::new();
    factory.connect_setup(|_, header| {
        if let Some(header) = header.downcast_ref::<gtk::ListHeader>() {
            header.set_child(Some(&CardGroupHeader::default()));
        }
    });
    let groups = groups.downgrade();
    factory.connect_bind(move |_, header| {
        let Some(header) = header.downcast_ref::<gtk::ListHeader>() else {
            return;
        };
        let (Some(groups), Some(widget)) = (
            groups.upgrade(),
            header.child().and_downcast::<CardGroupHeader>(),
        ) else {
            return;
        };
        if let Some(group) = groups.group_at(header.start()) {
            let names: Vec<String> = group
                .tags
                .iter()
                .map(|t| crate::ui::text::one_line(&t.name))
                .collect();
            widget.show(&names, groups.has_shared(), group.len);
        }
    });
    factory
}

impl SelectionPage for DeckPage {
    fn selection_mode_changed(&self, active: bool) {
        // The rows follow it; the action bar takes the Study bar's place.
        self.set_selecting(active);
        self.update_study_bar();
    }

    fn cards_changed(&self) {
        // Moved and deleted cards leave the list; the counts, the deck list
        // and All Cards follow.
        self.reload();
        let window = self.root().and_downcast::<RappelWindow>();
        counts_changed(self, window.as_ref());
    }

    fn reread_cards(&self) {
        self.reload();
    }

    fn list_shown(&self) -> bool {
        self.imp().content_stack.visible_child_name().as_deref() == Some("list")
    }

    fn focus_empty_state(&self) {
        self.focus_or_empty_state(None);
    }

    fn can_move_cards(&self) -> bool {
        self.can_move()
    }
}

#[cfg(test)]
mod tests {
    use rappel_core::domain::Grade;
    use rappel_core::scheduling::Due;

    use super::*;
    use crate::ui::selection::Bulk;

    fn due(id: &str) -> Due {
        Due {
            card_id: id.to_owned(),
            due_at: 0,
        }
    }

    #[test]
    fn the_new_flags_change_in_one_range() {
        assert_eq!(changed_range::<bool>(&[]), None);
        assert_eq!(changed_range(&[(true, true), (false, false)]), None);
        assert_eq!(changed_range(&[(true, false)]), Some(0..1));
        assert_eq!(
            changed_range(&[
                (true, true),
                (false, true),
                (true, true),
                (true, false),
                (false, false)
            ]),
            Some(1..4)
        );
    }

    #[test]
    fn the_timer_waits_for_the_next_due_card_or_the_next_day() {
        let now = 1_000_000;
        // A card due in 10 minutes comes before tomorrow.
        assert_eq!(
            timer_delay_ms(now, Some(now + 600_000), now + 50_000_000),
            600_000
        );
        // Nothing scheduled: wait for the next day, at most an hour at a time.
        assert_eq!(timer_delay_ms(now, None, now + 1_800_000), 1_800_000);
        assert_eq!(
            timer_delay_ms(now, None, now + 50_000_000),
            MAX_STUDY_WAIT_MS
        );
        // The next day comes before the card.
        assert_eq!(
            timer_delay_ms(now, Some(now + 900_000), now + 300_000),
            300_000
        );
        // A time already past fires at once.
        assert_eq!(timer_delay_ms(now, Some(now - 5), now + 300_000), 0);
    }

    #[test]
    fn finding_the_last_of_5000_cards_is_fast() {
        // About 2 ms of CPU time in a debug build; the bound leaves 10 times
        // that, and more than the CPU clock's precision (one tick).
        let bound = Duration::from_millis(20);
        let (found, scans, verdict) = crate::testing::within(
            Duration::from_secs(5),
            "5,000 cards made and scanned",
            move || {
                let store = gio::ListStore::new::<CardObject>();
                let items: Vec<CardObject> = (0..5_000)
                    .map(|i| {
                        CardObject::new(TaggedCard {
                            card: Card {
                                id: format!("card-{i:05}"),
                                deck_id: "deck".to_owned(),
                                front_md: "Front".to_owned(),
                                back_md: "Back".to_owned(),
                                created_at: i,
                                updated_at: i,
                                suspended: false,
                            },
                            tags: Vec::new(),
                        })
                    })
                    .collect();
                store.extend_from_slice(&items);
                let (scans, found) = crate::testing::cpu_time_of(|| {
                    (
                        position_in(&store, "card-04999"),
                        position_in(&store, "missing"),
                    )
                });
                // Checked on this thread, which did the work (ADR 0049).
                (found, scans, crate::testing::cpu_within(scans, bound))
            },
        );
        crate::testing::print_timing("position_in, two scans of 5,000 cards", scans);
        assert_eq!(found, (Some(4_999), None));
        if let Err(message) = verdict {
            panic!("position_in, two scans of 5,000 cards: {message}");
        }
    }

    #[test]
    fn the_study_bar_shows_only_on_a_narrow_window_with_cards() {
        assert!(study_bar_shown(true, false));
        assert!(!study_bar_shown(true, true));
        assert!(!study_bar_shown(false, false));
        assert!(!study_bar_shown(false, true));
    }

    #[test]
    fn subtitle_names_what_is_left() {
        let mut input = StudyInput::default();
        assert_eq!(study_subtitle(&input, false), "Nothing to study");
        input.new = vec!["n".to_owned(); 20];
        assert_eq!(study_subtitle(&input, false), "20 new");
        input.reviews = vec![due("r1"), due("r2")];
        input.steps = vec![due("s")];
        assert_eq!(study_subtitle(&input, false), "3 due · 20 new");
        input.new.clear();
        assert_eq!(study_subtitle(&input, false), "3 due");
        // Digits are grouped, as in the deck list.
        input.reviews = vec![due("r"); 1_234];
        assert_eq!(study_subtitle(&input, false), "1,235 due");
    }

    #[test]
    fn a_phone_subtitle_keeps_one_count() {
        let mut input = StudyInput::default();
        assert_eq!(study_subtitle(&input, true), "Nothing to study");
        input.new = vec!["n".to_owned(); 20];
        assert_eq!(study_subtitle(&input, true), "20 new");
        input.reviews = vec![due("r1"), due("r2")];
        input.steps = vec![due("s")];
        assert_eq!(study_subtitle(&input, true), "3 due");
        assert_eq!(study_subtitle(&input, false), "3 due · 20 new");
    }

    /// A page over a deck of `n` cards that are all due.
    fn page_of(n: usize) -> (tempfile::TempDir, AppState, DeckPage, Vec<String>) {
        let (tmp, state) = crate::testing::temporary_state();
        let deck = state.create_deck("Deck").unwrap();
        let now = rappel_core::time::now_ms();
        let day = crate::ui::time::DAY_MS;
        let ids = (0..n)
            .map(|i| {
                let id = state
                    .add_card_with_tags(&deck.id, &format!("front {i}"), "back", &[])
                    .unwrap()
                    .card
                    .id;
                for at in [now - 60 * day, now - 60 * day + 11 * 60 * 1000] {
                    state
                        .record_review(&id, Grade::Pass, Duration::ZERO, at)
                        .unwrap();
                }
                id
            })
            .collect();
        let page = DeckPage::new(state.clone(), deck).unwrap();
        (tmp, state, page, ids)
    }

    #[test]
    fn a_reset_changes_the_counts_and_its_undo_changes_them_back() {
        crate::testing::with_widgets(|| {
            let (_tmp, state, page, ids) = page_of(2);
            let subtitle = || page.imp().window_title.subtitle().to_string();
            assert_eq!(subtitle(), "2 due");
            page.reset(&ids[0]);
            assert_eq!(subtitle(), "1 due · 1 new");
            // The reset is the card's latest entry: the toast's Undo works.
            let entry = state
                .review_events(&ids[0])
                .unwrap()
                .last()
                .unwrap()
                .id
                .clone();
            undo_reset(&state, &ids[0], &entry, &page.downgrade(), None);
            assert_eq!(subtitle(), "2 due");
        });
    }

    #[test]
    fn a_suspended_card_leaves_the_counts_and_its_undo_brings_it_back() {
        crate::testing::with_widgets(|| {
            let (_tmp, state, page, ids) = page_of(2);
            let subtitle = || page.imp().window_title.subtitle().to_string();
            assert_eq!(subtitle(), "2 due");
            page.suspend(&ids[0], true);
            assert_eq!(subtitle(), "1 due");
            assert!(page.item(0).unwrap().card().suspended);
            assert!(page.item(0).unwrap().row_label().ends_with(", suspended"));
            undo_suspend(&state, &ids[0], true, &page.downgrade(), None);
            assert_eq!(subtitle(), "2 due");
            assert!(!page.item(0).unwrap().card().suspended);
        });
    }

    #[test]
    fn move_to_deck_is_enabled_only_while_another_deck_exists() {
        crate::testing::with_widgets(|| {
            let (_tmp, state, page, _) = page_of(1);
            assert!(!page.can_move(), "the only deck");
            let other = state.create_deck("Other").unwrap();
            // The page reads again after an import or a change elsewhere.
            page.reload();
            assert!(page.can_move());
            state.delete_deck(&other.id).unwrap();
            page.reload();
            assert!(!page.can_move());
        });
    }

    #[test]
    fn a_moved_card_leaves_the_list_and_its_undo_brings_it_back() {
        crate::testing::with_widgets(|| {
            let (_tmp, state, page, ids) = page_of(3);
            let other = state.create_deck("Other").unwrap();
            let from = page.deck_id().unwrap();
            let subtitle = || page.imp().window_title.subtitle().to_string();
            assert_eq!(subtitle(), "3 due");
            let list = || page.imp().cards.get().unwrap().n_items();

            // The middle card goes: the focus is for the one now at its place.
            let focus = page.finish_move(&state, &ids[1], &other);
            assert_eq!(focus, Some(MovedFocus::Row(1)));
            assert_eq!(list(), 2);
            assert!(page.position(&ids[1]).is_none());
            assert_eq!(state.card(&ids[1]).unwrap().deck_id, other.id);
            assert_eq!(subtitle(), "2 due");

            // The last one goes: the focus is for the new last row.
            assert_eq!(
                page.finish_move(&state, &ids[2], &other),
                Some(MovedFocus::Row(0))
            );
            // The only one goes: the empty state.
            assert_eq!(
                page.finish_move(&state, &ids[0], &other),
                Some(MovedFocus::EmptyState)
            );
            assert_eq!(list(), 0);
            assert_eq!(
                page.imp().content_stack.visible_child_name().as_deref(),
                Some("empty")
            );

            // Undo of the first move: back in its place.
            let moved = vec![rappel_core::storage::Moved {
                card_id: ids[1].clone(),
                from: from.clone(),
                to: other.id.clone(),
            }];
            undo_card_move(
                &state,
                &ids[1],
                state.unmove_cards(&moved),
                &page.downgrade(),
                None,
            );
            assert_eq!(state.card(&ids[1]).unwrap().deck_id, from);
            assert_eq!(list(), 1);
            assert!(page.position(&ids[1]).is_some());

            // An Undo that could not move the card back adds nothing.
            let gone = vec![rappel_core::storage::Moved {
                card_id: ids[2].clone(),
                from: from.clone(),
                to: "another".to_owned(),
            }];
            undo_card_move(
                &state,
                &ids[2],
                state.unmove_cards(&gone),
                &page.downgrade(),
                None,
            );
            assert_eq!(list(), 1);
            assert_eq!(state.card(&ids[2]).unwrap().deck_id, other.id);
        });
    }

    #[test]
    fn a_card_that_is_gone_or_already_there_is_not_moved_twice() {
        crate::testing::with_widgets(|| {
            let (_tmp, state, page, ids) = page_of(2);
            let other = state.create_deck("Other").unwrap();
            // Moved behind the page's back: the list is stale.
            state
                .move_cards(std::slice::from_ref(&ids[0]), &other.id)
                .unwrap();
            assert_eq!(page.finish_move(&state, &ids[0], &other), None);
            assert_eq!(page.imp().cards.get().unwrap().n_items(), 1, "read again");
            // Deleted behind the page's back.
            state.delete_card(&ids[1]).unwrap();
            assert_eq!(page.finish_move(&state, &ids[1], &other), None);
            assert_eq!(page.imp().cards.get().unwrap().n_items(), 0);
        });
    }

    #[test]
    fn the_study_bar_follows_the_width_and_the_cards() {
        crate::testing::with_widgets(|| {
            let (_tmp, state, page, ids) = page_of(1);
            let bar = || page.imp().study_bar.is_revealed();
            assert!(!bar());
            page.set_compact(true);
            assert!(bar());
            // The last card goes: the empty state stands alone.
            page.delete(&ids[0]);
            assert!(!bar());
            undo_card_delete(&state, &ids[0], &page.downgrade(), None);
            assert!(bar());
            page.set_compact(false);
            assert!(!bar());
        });
    }

    #[test]
    fn a_reset_of_a_new_card_is_a_message_and_other_failures_are_errors() {
        assert_eq!(
            reset_notice(&Error::CardAlreadyNew("c".to_owned())),
            Notice::Info("This card has no progress to reset".to_owned())
        );
        assert!(matches!(
            reset_notice(&Error::CardNotFound("c".to_owned())),
            Notice::Error(_)
        ));
    }

    #[test]
    fn a_row_offers_reset_only_while_the_card_has_progress() {
        crate::testing::with_widgets(|| {
            let (_tmp, state, page, ids) = page_of(1);
            let is_new = |page: &DeckPage| page.item(0).unwrap().is_new();
            assert!(!is_new(&page), "studied cards start with progress");
            page.reset(&ids[0]);
            assert!(is_new(&page), "a reset card is new");
            let entry = state.review_events(&ids[0]).unwrap().pop().unwrap().id;
            undo_reset(&state, &ids[0], &entry, &page.downgrade(), None);
            assert!(!is_new(&page), "undoing the reset brings the progress back");

            // Studied elsewhere after a reset: `shown` catches up.
            page.reset(&ids[0]);
            assert!(is_new(&page));
            let now = rappel_core::time::now_ms();
            state
                .record_review(&ids[0], Grade::Pass, Duration::ZERO, now)
                .unwrap();
            page.emit_by_name::<()>("shown", &[]);
            assert!(!is_new(&page));
        });
    }

    #[test]
    fn a_stale_reset_of_a_new_card_writes_nothing_and_updates_the_row() {
        crate::testing::with_widgets(|| {
            let (_tmp, state, page, ids) = page_of(1);
            // Reset behind the page's back: its row is stale.
            state
                .reset_card(&ids[0], rappel_core::time::now_ms())
                .unwrap();
            assert!(!page.item(0).unwrap().is_new());
            let log = state.review_events(&ids[0]).unwrap();
            // The stale menu's reset is refused and the row catches up.
            page.reset(&ids[0]);
            assert_eq!(state.review_events(&ids[0]).unwrap(), log);
            assert!(page.item(0).unwrap().is_new());
        });
    }

    #[test]
    fn an_added_card_is_new_and_an_edit_keeps_the_progress() {
        crate::testing::with_widgets(|| {
            let (_tmp, state, page, ids) = page_of(1);
            let edited = state
                .edit_card_with_tags(&ids[0], "new front", "back", None)
                .unwrap();
            page.show_saved(Saved::Edited(edited));
            assert!(!page.item(0).unwrap().is_new());
            let deck = page.deck_id().unwrap();
            let added = state.add_card_with_tags(&deck, "q", "a", &[]).unwrap();
            page.show_saved(Saved::Added(added));
            assert!(page.item(1).unwrap().is_new());
        });
    }

    /// The tags every card of the deck has are said once above the list,
    /// and each row shows only its others (ADR 0077); a card added without
    /// them ends that.
    #[test]
    fn the_tags_every_card_has_are_said_once_above_the_list() {
        crate::testing::with_widgets(|| {
            let (_tmp, state) = crate::testing::temporary_state();
            let deck = state.create_deck("Deck").unwrap();
            let tags =
                |names: &[&str]| -> Vec<String> { names.iter().map(|&n| n.to_owned()).collect() };
            for (front, names) in [
                ("one", tags(&["def", "maths"])),
                ("two", tags(&["def", "hard", "maths"])),
                ("three", tags(&["maths", "def"])),
            ] {
                state
                    .add_card_with_tags(&deck.id, front, "back", &names)
                    .unwrap();
            }
            let page = DeckPage::new(state.clone(), deck.clone()).unwrap();
            assert_eq!(page.shared_tag_names(), ["def", "maths"]);
            let window = presented(&page);
            crate::testing::settle();
            let list = page.imp().card_list.upcast_ref::<gtk::Widget>();
            let header = &page.imp().list_header;
            assert!(header.is_visible());
            assert_eq!(header.tags(), ["def", "maths"]);
            // Two groups (ADR 0079): the cards with only the shared tags,
            // then the one with "hard" too, under headers; no pills.
            assert!(page.grouped());
            assert!(page.imp().card_list.header_factory().is_some());
            let shown: Vec<String> = (0..3)
                .map(|p| page.shown_item(p).unwrap().card().front_md.clone())
                .collect();
            assert_eq!(shown, ["one", "three", "two"]);
            assert!(
                rows(list)
                    .iter()
                    .all(|row| !row.imp().tag_pills.is_visible())
            );
            let headers: Vec<CardGroupHeader> = crate::testing::descendants(list);
            assert_eq!(headers.len(), 2);
            let said: Vec<String> = headers
                .iter()
                .filter_map(|h| {
                    crate::testing::label_texts(h.upcast_ref())
                        .into_iter()
                        .filter(|t| !t.is_empty())
                        .reduce(|a, b| format!("{a} {b}"))
                })
                .collect();
            assert!(
                said.contains(&"Only the shared tags 2 cards".to_owned()),
                "{said:?}"
            );
            assert!(said.contains(&"Also hard 1 card".to_owned()), "{said:?}");
            // A card without the tags: nothing is shared any more, the
            // groups would mostly hold one card, so plain rows show all
            // their tags again.
            let added = state
                .add_card_with_tags(&deck.id, "four", "a", &[])
                .unwrap();
            page.show_saved(Saved::Added(added));
            crate::testing::settle();
            assert_eq!(page.shared_tag_names(), Vec::<String>::new());
            assert_eq!(header.tags(), Vec::<String>::new());
            assert!(!header.is_visible());
            assert!(!page.grouped());
            assert!(page.imp().card_list.header_factory().is_none());
            let first = rows(list)
                .into_iter()
                .find(|row| row.card_id() == page.item(0).unwrap().card().id)
                .unwrap();
            assert_eq!(first.imp().tag_pills.tags(), ["def", "maths"]);
            window.destroy();
        });
    }

    /// A deck of `n` cards sharing "def" and "maths", every third with
    /// "hard" too: two groups, the second after the first.
    fn grouped_page(n: usize) -> (tempfile::TempDir, AppState, DeckPage, Vec<String>) {
        let (tmp, state) = crate::testing::temporary_state();
        let deck = state.create_deck("Deck").unwrap();
        let ids = (0..n)
            .map(|i| {
                let mut tags = vec!["def".to_owned(), "maths".to_owned()];
                if i % 3 == 0 {
                    tags.push("hard".to_owned());
                }
                state
                    .add_card_with_tags(&deck.id, &format!("card {i}"), "", &tags)
                    .unwrap()
                    .card
                    .id
            })
            .collect();
        let page = DeckPage::new(state.clone(), deck).unwrap();
        (tmp, state, page, ids)
    }

    /// Opening a grouped deck focuses its first row with the first group's
    /// header still in sight above it (the list scrolled it away before,
    /// ADR 0077), scrolls are never fought, and the page's own focus on
    /// the first card from far down brings the header back (ADR 0079).
    #[test]
    fn the_first_groups_header_stays_in_sight() {
        crate::testing::with_widgets(|| {
            let (_tmp, _state, page, _ids) = grouped_page(60);
            assert!(page.grouped());
            let view = adw::NavigationView::new();
            let window = gtk::Window::builder()
                .default_width(600)
                .default_height(500)
                .child(&view)
                .build();
            window.present();
            view.push(&page);
            let list = page.imp().card_list.get();
            let adjustment = list.vadjustment().unwrap();
            let first = page.shown_item(0).unwrap().card().id.clone();
            let settled = crate::testing::settle_until(
                || focused_card(&window).as_ref() == Some(&first) && adjustment.upper() > 0.0,
                Duration::from_secs(10),
            );
            assert!(settled, "the first row is focused");
            // The row was focused once laid out, so the list is still at
            // the very top with the header in view.
            let header_shown = || {
                list.first_child()
                    .is_some_and(|h| h.css_name() == "header" && h.height() > 0)
            };
            let top = crate::testing::settle_until(
                || adjustment.value() == 0.0 && header_shown(),
                Duration::from_secs(10),
            );
            assert!(top, "scrolled to {}", adjustment.value());
            // It stays there.
            crate::testing::settle_until(|| false, Duration::from_millis(300));
            assert_eq!(adjustment.value(), 0.0);

            // A scroll from the top (the wheel, the scroll bar), even one
            // that stops with the focused first row exactly at the top, is
            // left alone: nothing watches the scroll.
            let header_height = list
                .first_child()
                .and_then(|h| h.compute_bounds(&list))
                .map_or(0.0, |b| f64::from(b.height()));
            assert!(header_height > 0.0);
            adjustment.set_value(header_height);
            crate::testing::settle_until(|| false, Duration::from_millis(500));
            assert_eq!(
                adjustment.value(),
                header_height,
                "a scroll was snapped back"
            );

            // A scroll down from the top is left alone.
            adjustment.set_value(10.0);
            crate::testing::settle_until(|| false, Duration::from_millis(300));
            assert_eq!(adjustment.value(), 10.0);

            // Far down the list, the page gives the first card the focus
            // (the Undo of its delete, the editor closing on it): the
            // header comes into view with it, as on open.
            adjustment.set_value(900.0);
            let away = crate::testing::settle_until(
                || adjustment.value() > 100.0,
                Duration::from_secs(10),
            );
            assert!(away, "not scrolled away: {}", adjustment.value());
            page.focus_card(&first);
            let back = crate::testing::settle_until(
                || {
                    focused_card(&window).as_ref() == Some(&first)
                        && adjustment.value() == 0.0
                        && header_shown()
                },
                Duration::from_secs(10),
            );
            assert!(
                back,
                "focus on {:?}, scrolled to {}",
                focused_card(&window),
                adjustment.value()
            );
            // Not covered here: keys delivered by a real keyboard. Up, Page
            // Up or Home onto the first row from further down scroll it
            // just into view with its header above, as any sectioned GTK
            // list does; Tab back into the list focuses the row it had,
            // which GTK does not scroll to (`gtk_list_base_set_focus_child`
            // scrolls only to a new focus position).
            window.destroy();
        });
    }

    /// In a grouped list, a click opens the card shown there, and after a
    /// delete the focus goes to the row now at the deleted row's place in
    /// the list shown (`index_after_removal`), not in creation order.
    #[test]
    fn a_grouped_list_acts_on_the_rows_shown() {
        crate::testing::with_widgets(|| {
            let (_tmp, _state, page, ids) = grouped_page(9);
            // Shown: 1 2 4 5 7 8, then 0 3 6 (with "hard").
            let shown: Vec<String> = (0..9)
                .map(|p| page.shown_item(p).unwrap().card().id.clone())
                .collect();
            let order = [1, 2, 4, 5, 7, 8, 0, 3, 6].map(|i| ids[i].clone());
            assert_eq!(shown, order);
            let window = presented(&page);
            crate::testing::settle();
            // Card 0 is shown at 6: deleting it focuses card 3, now at 6.
            page.delete(&ids[0]);
            let focused = crate::testing::settle_until(
                || focused_card(&window).as_ref() == Some(&ids[3]),
                Duration::from_secs(5),
            );
            assert!(focused, "{:?}", focused_card(&window));
            // The groups' headers follow: the "hard" group lost a card.
            let counts = || -> Vec<String> {
                crate::testing::descendants::<CardGroupHeader>(page.upcast_ref())
                    .iter()
                    .flat_map(|h| crate::testing::label_texts(h.upcast_ref()))
                    .filter(|t| t.ends_with("cards") || t.ends_with("card"))
                    .collect()
            };
            crate::testing::settle();
            assert_eq!(counts(), ["6 cards", "2 cards"]);
            // Choosing by a click on the row shown at 0 chooses card 1.
            page.selection().set_active(true);
            assert!(page.selection().activate(0));
            assert_eq!(page.selection().chosen_ids(), [ids[1].clone()]);
            // Section headers stay while choosing; the rows have checks.
            assert!(page.imp().card_list.header_factory().is_some());
            crate::testing::settle();
            assert!(
                rows(page.upcast_ref())
                    .iter()
                    .all(|r| r.imp().check.is_visible())
            );
            window.destroy();
        });
    }

    /// A card's tag starts a session over that tag from this page (ADR
    /// 0069); a save from it of another deck's card leaves this list alone.
    #[test]
    fn a_cards_tag_starts_a_session_over_it_and_other_decks_stay_out() {
        crate::testing::with_widgets(|| {
            let (_tmp, state, page, ids) = page_of(1);
            let other = state.create_deck("Other").unwrap();
            let tagged = state
                .add_card_with_tags(&other.id, "elsewhere", "a", &["verbs".to_owned()])
                .unwrap();
            let view = adw::NavigationView::new();
            let window = gtk::Window::builder().child(&view).build();
            view.push(&page);
            let tag = tagged.tags[0].id.clone();
            page.activate_action("deck.study-tag", Some(&tag.to_variant()))
                .unwrap();
            let review = view
                .visible_page()
                .and_downcast::<ReviewPage>()
                .expect("a session is pushed");
            assert_eq!(review.deck_id(), None);
            // An unknown tag starts nothing.
            view.pop();
            page.activate_action(
                "deck.study-tag",
                Some(&"01920000-0000-7000-8000-00000000dead".to_variant()),
            )
            .unwrap();
            assert!(view.visible_page().and_downcast::<ReviewPage>().is_none());
            // The other deck's card, saved from the session's editor.
            let edited = state
                .edit_card_with_tags(&tagged.card.id, "edited", "a", None)
                .unwrap();
            assert_eq!(page.show_saved(Saved::Edited(edited)), None);
            let listed: Vec<String> = (0..page.imp().cards.get().unwrap().n_items())
                .map(|p| page.item(p).unwrap().card().id.clone())
                .collect();
            assert_eq!(listed, ids);
            window.destroy();
        });
    }

    /// The Deck Menu lists the tags of the deck's cards in a "Study Cards
    /// Tagged" submenu after Statistics, each studying its tag, and follows
    /// the list: no tag, no submenu (ADR 0069).
    #[test]
    fn the_deck_menu_offers_to_study_the_decks_tags() {
        crate::testing::with_widgets(|| {
            let (_tmp, state, page, ids) = page_of(2);
            let section = page.imp().tag_section.get().unwrap().clone();
            let menu = page.imp().deck_menu.get();
            assert_eq!(
                menu.item_link(1, "section"),
                Some(section.clone().upcast::<gio::MenuModel>())
            );
            page.update_tag_menu();
            assert_eq!(section.n_items(), 0, "no tag yet");
            let deck = page.deck_id().unwrap();
            let tagged = state
                .add_card_with_tags(&deck, "q", "a", &["zeta".to_owned(), "alpha".to_owned()])
                .unwrap();
            page.show_saved(Saved::Added(tagged.clone()));
            // Filled when the menu opens, not at the change.
            assert_eq!(section.n_items(), 0);
            let window = gtk::Window::builder().child(&page).build();
            window.present();
            page.imp().deck_menu_button.set_active(true);
            assert_eq!(section.n_items(), 1);
            page.imp().deck_menu_button.set_active(false);
            let submenu = section.item_link(0, "submenu").unwrap();
            let entry = |i: i32, key: &str| {
                submenu
                    .item_attribute_value(i, key, Some(glib::VariantTy::STRING))
                    .and_then(|v| v.get::<String>())
                    .unwrap()
            };
            assert_eq!(submenu.n_items(), 2);
            assert_eq!(entry(0, "label"), "\u{2068}alpha\u{2069}");
            assert_eq!(entry(1, "label"), "\u{2068}zeta\u{2069}");
            assert_eq!(entry(0, "action"), "deck.study-tag");
            let alpha = tagged.tags.iter().find(|t| t.name == "alpha").unwrap();
            assert_eq!(entry(0, "target"), alpha.id);
            // The card goes: so does the submenu.
            page.delete(&tagged.card.id);
            page.update_tag_menu();
            assert_eq!(section.n_items(), 0);
            // Unchanged since: opening again reads nothing.
            assert!(!page.imp().tag_menu_stale.get());
            window.destroy();
            assert_eq!(ids.len(), 2);
        });
    }

    #[test]
    fn an_undo_after_the_card_was_studied_again_changes_nothing() {
        crate::testing::with_widgets(|| {
            let (_tmp, state, page, ids) = page_of(1);
            page.reset(&ids[0]);
            let reset = state
                .review_events(&ids[0])
                .unwrap()
                .last()
                .unwrap()
                .id
                .clone();
            let now = rappel_core::time::now_ms();
            state
                .record_review(&ids[0], Grade::Pass, Duration::ZERO, now)
                .unwrap();
            let log = state.review_events(&ids[0]).unwrap();
            let card_state = state.card_state(&ids[0]).unwrap();
            undo_reset(&state, &ids[0], &reset, &page.downgrade(), None);
            assert_eq!(state.review_events(&ids[0]).unwrap(), log);
            assert_eq!(state.card_state(&ids[0]).unwrap(), card_state);
        });
    }

    // --- selection mode (#104) ---------------------------------------------

    /// `page` in a presented window: in a window that never was, a hidden
    /// widget stays hidden when shown (observed with GTK 4.24.1), and the
    /// header of selection mode must show.
    fn presented(page: &DeckPage) -> gtk::Window {
        let window = gtk::Window::builder()
            .default_width(800)
            .default_height(600)
            .child(page)
            .build();
        window.present();
        window
    }

    /// The cards listed, in order.
    fn listed(page: &DeckPage) -> Vec<String> {
        let n = page.imp().cards.get().unwrap().n_items();
        (0..n)
            .map(|p| page.item(p).unwrap().card().id.clone())
            .collect()
    }

    /// The card rows built, wherever they are.
    fn rows(widget: &gtk::Widget) -> Vec<CardRow> {
        crate::testing::descendants(widget)
    }

    /// The card whose row has the window's focus.
    fn focused_card(window: &gtk::Window) -> Option<String> {
        let focus = gtk::prelude::GtkWindowExt::focus(window)?;
        let row = focus
            .first_child()
            .and_downcast::<CardRow>()
            .or_else(|| focus.downcast::<CardRow>().ok())?;
        Some(row.card_id())
    }

    /// Presses Ctrl+A on the page's controller (`ui::select_all`), as a key
    /// press that reached the page would. Returns whether it was handled.
    fn press_ctrl_a(page: &DeckPage) -> bool {
        use gtk::gdk::ModifierType;
        let controller = page
            .observe_controllers()
            .iter::<glib::Object>()
            .filter_map(Result::ok)
            .filter_map(|c| c.downcast::<gtk::EventControllerKey>().ok())
            .find(|c| c.name().as_deref() == Some(crate::ui::select_all::CONTROLLER_NAME))
            .expect("the page installs its Ctrl+A controller");
        controller.emit_by_name::<bool>(
            "key-pressed",
            &[&gtk::gdk::Key::a, &0u32, &ModifierType::CONTROL_MASK],
        )
    }

    /// At 360 × 640 Select Cards is in the bottom bar, not the header
    /// (`select_slot` hides), and leaving selection mode from Cancel gives
    /// the keyboard to that bar button, not to the hidden one (ui/selection.rs).
    #[test]
    fn on_a_phone_cancel_gives_the_focus_to_the_select_button_of_the_bar() {
        crate::testing::with_widgets(|| {
            let (_tmp, _state, page, _ids) = page_of(3);
            let window = gtk::Window::builder()
                .default_width(360)
                .default_height(640)
                .child(&page)
                .build();
            window.present();
            let imp = page.imp();
            assert!(
                crate::testing::settle_until(
                    || page.compact() && imp.study_bar.is_revealed(),
                    Duration::from_secs(5)
                ),
                "the phone layout applies"
            );
            assert!(!imp.select_button.is_visible(), "not in the header");
            assert!(imp.bar_select_button.is_visible());
            imp.bar_select_button.emit_clicked();
            assert!(page.selecting());
            let cancel = imp.selection_header.cancel_button();
            gtk::prelude::GtkWindowExt::set_focus(&window, Some(&cancel));
            cancel.emit_clicked();
            assert!(!page.selecting());
            assert!(
                crate::testing::settle_until(
                    || imp.bar_select_button.has_focus(),
                    Duration::from_secs(5)
                ),
                "the focus is on the bar's Select button"
            );
            window.destroy();
        });
    }

    #[test]
    fn select_cards_enters_selection_mode_and_cancel_or_escape_leaves_it() {
        crate::testing::with_widgets(|| {
            let (_tmp, _state, page, ids) = page_of(3);
            let window = presented(&page);
            let imp = page.imp();
            let selection = page.selection();
            assert!(page.can_pop() && imp.header.is_visible());
            gtk::prelude::GtkWindowExt::set_focus(&window, Some(&*imp.select_button));
            imp.select_button.emit_clicked();
            assert!(page.selecting() && selection.active());
            assert!(imp.selection_header.is_visible() && !imp.header.is_visible());
            assert!(selection.bar().is_revealed());
            assert!(
                !page.can_pop(),
                "Escape leaves selection mode, not the page"
            );
            assert_eq!(selection.header().title(), "Select Cards");
            assert!(
                gtk::prelude::GtkWindowExt::focus(&window)
                    .is_some_and(|w| w == *imp.card_list || w.is_ancestor(&*imp.card_list)),
                "the focus went to the list, not nowhere"
            );
            // The rows show their check button instead of their menu.
            let built = rows(page.upcast_ref());
            assert!(!built.is_empty() && built.iter().all(CardRow::selecting));

            // A click on a row chooses it rather than opening the editor.
            imp.card_list.emit_by_name::<()>("activate", &[&1u32]);
            assert!(!imp.editing.get());
            assert_eq!(selection.chosen_ids(), [ids[1].clone()]);
            assert_eq!(selection.header().title(), "1 Selected");
            assert!(page.item(1).unwrap().selected());
            // Ctrl+N does not open the editor while choosing.
            page.new_card();
            assert!(!imp.editing.get());

            imp.selection_header.cancel_button().emit_clicked();
            assert!(!page.selecting() && !selection.active());
            assert!(page.can_pop() && imp.header.is_visible());
            assert!(!imp.selection_header.is_visible());
            assert!(!page.item(1).unwrap().selected(), "the choice is forgotten");

            // Escape leaves the mode, then is left to the navigation view.
            // (A long press starts choosing with its card.)
            page.activate_action("deck.select-card", Some(&ids[0].to_variant()))
                .unwrap();
            assert!(selection.active());
            assert_eq!(selection.chosen_ids(), [ids[0].clone()]);
            assert_eq!(page.escape(), glib::Propagation::Stop);
            assert!(!selection.active() && page.can_pop());
            assert_eq!(page.escape(), glib::Propagation::Proceed);
            window.destroy();
        });
    }

    #[test]
    fn ctrl_a_chooses_every_card_of_the_deck() {
        crate::testing::with_widgets(|| {
            let (_tmp, _state, page, ids) = page_of(3);
            let window = presented(&page);
            gtk::prelude::GtkWindowExt::set_focus(&window, Some(&*page.imp().card_list));
            assert!(press_ctrl_a(&page));
            let selection = page.selection();
            assert!(selection.active());
            assert_eq!(selection.chosen_ids(), ids);
            assert_eq!(selection.header().title(), "3 Selected");
            window.destroy();

            // An empty deck: nothing to choose, and Select is disabled.
            let (_tmp, state) = crate::testing::temporary_state();
            let deck = state.create_deck("Empty").unwrap();
            let page = DeckPage::new(state, deck).unwrap();
            let window = presented(&page);
            assert!(!page.imp().select_button.is_sensitive());
            let _ = press_ctrl_a(&page);
            assert!(!page.selection().active());
            window.destroy();
        });
    }

    /// Hardening round 4 (ADR 0073): Ctrl+A on a deck of 10,000 cards.
    /// Choosing them all is quick (CPU time, ADR 0049), the header counts
    /// them, the list still builds only the rows on screen (choosing binds
    /// no widget per card), and Suspend on all of them and its Undo work.
    /// Then the new-card flags of every row change at once (as after a
    /// reset of them all) while cards are chosen: one splice, so the
    /// counts and the selection follow it once, not once per row (slice
    /// 2's review: 10,000 splices, each walking the list and reading the
    /// study counts, froze the window).
    #[test]
    fn ctrl_a_on_ten_thousand_cards_is_quick_and_its_bulk_action_undoes() {
        crate::testing::with_widgets(|| {
            const N: usize = 10_000;
            let (_tmp, state, deck, ids) = crate::testing::state_with_many_cards(N);
            let page = DeckPage::new(state.clone(), deck).unwrap();
            let window = presented(&page);
            let settle = crate::testing::settle;
            settle();
            // GTK builds the rows on screen and a margin around them (about
            // 200 here), never one per card.
            let built = rows(page.upcast_ref()).len();
            assert!((1..1_000).contains(&built), "{built} rows");
            gtk::prelude::GtkWindowExt::set_focus(&window, Some(&*page.imp().card_list));

            let (choose, pressed) = crate::testing::cpu_time_of(|| press_ctrl_a(&page));
            assert!(pressed);
            let selection = page.selection();
            assert_eq!(selection.chosen_ids().len(), N);
            assert_eq!(selection.header().title(), "10,000 Selected");
            settle();
            assert!(rows(page.upcast_ref()).len() <= built, "no row per card");

            let chosen = selection.chosen_ids();
            let (suspend, _) =
                crate::testing::cpu_time_of(|| selection.run(&chosen, Bulk::Suspend(true)));
            assert!(!selection.active(), "selection mode ends");
            assert!(state.card(&ids[N - 1]).unwrap().suspended);
            assert!((0..N).all(|p| {
                page.item(u32::try_from(p).unwrap())
                    .unwrap()
                    .card()
                    .suspended
            }));
            let (undo, ()) = crate::testing::cpu_time_of(|| selection.undo_last());
            assert!(!state.card(&ids[0]).unwrap().suspended);
            assert!((0..N).all(|p| {
                !page
                    .item(u32::try_from(p).unwrap())
                    .unwrap()
                    .card()
                    .suspended
            }));

            // Every row stale at once, with cards chosen.
            let store = page.imp().cards.get().unwrap();
            let not_new: Vec<CardObject> = (0..store.n_items())
                .map(|p| {
                    page.item(p).unwrap().copy_with(CardStanding {
                        status: CardStatus::Due(0),
                        is_new: false,
                    })
                })
                .collect();
            store.splice(0, store.n_items(), &not_new);
            selection.set_active(true);
            selection.toggle(&ids[3]);
            let (refresh, ()) = crate::testing::cpu_time_of(|| page.refresh_standings());
            assert!((0..N).all(|p| page.item(u32::try_from(p).unwrap()).unwrap().is_new()));
            assert_eq!(selection.chosen_ids(), [ids[3].clone()]);
            window.destroy();

            crate::testing::print_timing("deck page: Ctrl+A on 10,000", choose);
            crate::testing::print_timing("deck page: suspend 10,000", suspend);
            crate::testing::print_timing("deck page: undo the suspend", undo);
            crate::testing::print_timing("deck page: 10,000 new flags", refresh);
            crate::testing::assert_cpu_within(choose, Duration::from_secs(1), "Ctrl+A");
            crate::testing::assert_cpu_within(suspend, Duration::from_secs(5), "suspend");
            crate::testing::assert_cpu_within(undo, Duration::from_secs(5), "undo");
            crate::testing::assert_cpu_within(refresh, Duration::from_secs(2), "new flags");
        });
    }

    #[test]
    fn each_bulk_action_changes_the_list_and_its_undo_restores_it() {
        crate::testing::with_widgets(|| {
            let (_tmp, state, page, ids) = page_of(3);
            let other = state.create_deck("Other").unwrap();
            page.reload();
            // Rooted, so the Undo finds the page (as when it is shown).
            let window = gtk::Window::builder().child(&page).build();
            let selection = page.selection();
            let subtitle = || page.imp().window_title.subtitle().to_string();
            let choose = |chosen: &[&String]| {
                selection.set_active(true);
                for id in chosen {
                    selection.toggle(id);
                }
                selection.chosen_ids()
            };

            // Move: the cards leave the list; Undo brings them back.
            let chosen = choose(&[&ids[0], &ids[2]]);
            selection.run(
                &chosen,
                Bulk::Move {
                    deck_id: other.id.clone(),
                },
            );
            assert!(!selection.active(), "selection mode ends");
            assert_eq!(listed(&page), [ids[1].clone()]);
            assert_eq!(state.card(&ids[0]).unwrap().deck_id, other.id);
            assert_eq!(subtitle(), "1 due");
            selection.undo_last();
            assert_eq!(listed(&page), ids);
            assert_eq!(subtitle(), "3 due");

            // Delete.
            let chosen = choose(&[&ids[1]]);
            selection.run(&chosen, Bulk::Delete);
            assert_eq!(listed(&page), [ids[0].clone(), ids[2].clone()]);
            selection.undo_last();
            assert_eq!(listed(&page), ids);

            // Suspend, through the action bar's action.
            choose(&[&ids[0], &ids[1]]);
            page.activate_action("selection.suspend", None).unwrap();
            assert!(page.item(0).unwrap().card().suspended);
            assert!(page.item(1).unwrap().card().suspended);
            assert_eq!(subtitle(), "1 due");
            selection.undo_last();
            assert!(!page.item(0).unwrap().card().suspended);
            assert_eq!(subtitle(), "3 due");

            // Reset Progress.
            let chosen = choose(&[&ids[2]]);
            selection.run(&chosen, Bulk::Reset);
            assert!(page.item(2).unwrap().is_new());
            selection.undo_last();
            assert!(!page.item(2).unwrap().is_new());

            // Add Tags, then Remove Tag.
            let names = |position: u32| page.item(position).unwrap().tags().clone();
            let chosen = choose(&[&ids[0], &ids[1]]);
            selection.run(&chosen, Bulk::AddTags(vec!["exam".to_owned()]));
            assert_eq!(names(0), ["exam"]);
            assert_eq!(names(1), ["exam"]);
            assert_eq!(names(2), Vec::<String>::new());
            selection.undo_last();
            assert_eq!(names(0), Vec::<String>::new());
            selection.run(&chosen, Bulk::AddTags(vec!["exam".to_owned()]));
            let exam = state.card_tags(&ids[0]).unwrap().remove(0);
            let chosen = choose(&[&ids[0]]);
            selection.run(&chosen, Bulk::RemoveTag(exam));
            assert_eq!(names(0), Vec::<String>::new());
            assert_eq!(names(1), ["exam"]);
            selection.undo_last();
            assert_eq!(names(0), ["exam"]);
            window.destroy();
        });
    }

    #[test]
    fn after_a_bulk_removal_the_focus_follows_the_list() {
        crate::testing::with_widgets(|| {
            let (_tmp, _state, page, ids) = page_of(3);
            let window = presented(&page);
            let selection = page.selection();
            let settle = || {
                crate::testing::settle_until(|| false, Duration::from_millis(100));
            };
            settle();
            // The middle card goes: the focus is on the one now at its place.
            selection.set_active(true);
            selection.toggle(&ids[1]);
            selection.run(&selection.chosen_ids(), Bulk::Delete);
            // Waited for, not a fixed time: under load the list's rows and
            // the focus come a few frames later (seen flaky with 100 ms).
            let focus_on = |id: &String| {
                crate::testing::settle_until(
                    || focused_card(&window).as_ref() == Some(id),
                    Duration::from_secs(5),
                )
            };
            assert!(focus_on(&ids[2]), "{:?}", focused_card(&window));
            // The last card goes: the focus is on the new last one.
            selection.set_active(true);
            selection.toggle(&ids[2]);
            selection.run(&selection.chosen_ids(), Bulk::Delete);
            assert!(focus_on(&ids[0]), "{:?}", focused_card(&window));
            // Every card goes: the empty state, with the focus on Add Card.
            selection.select_all();
            selection.run(&selection.chosen_ids(), Bulk::Delete);
            let add = page.imp().empty_add_button.get().upcast::<gtk::Widget>();
            assert!(crate::testing::settle_until(
                || gtk::prelude::GtkWindowExt::focus(&window).as_ref() == Some(&add),
                Duration::from_secs(5)
            ));
            assert!(!selection.active() && !page.selecting());
            assert_eq!(
                page.imp().content_stack.visible_child_name().as_deref(),
                Some("empty")
            );
            assert_eq!(
                gtk::prelude::GtkWindowExt::focus(&window),
                Some(page.imp().empty_add_button.get().upcast())
            );
            assert!(
                !page.imp().select_button.is_sensitive(),
                "nothing to choose"
            );
            window.destroy();
        });
    }

    #[test]
    fn move_is_offered_only_with_another_deck() {
        crate::testing::with_widgets(|| {
            let (_tmp, state, page, ids) = page_of(2);
            let window = presented(&page);
            let selection = page.selection();
            selection.set_active(true);
            selection.toggle(&ids[0]);
            let move_button = &selection.bar().buttons()[0];
            assert!(!move_button.is_sensitive(), "the only deck");
            state.create_deck("Other").unwrap();
            page.reload();
            assert!(move_button.is_sensitive());
            window.destroy();
        });
    }

    #[test]
    fn the_action_bar_takes_the_study_bars_place() {
        crate::testing::with_widgets(|| {
            let (_tmp, _state, page, _) = page_of(2);
            let bar = || page.imp().study_bar.is_revealed();
            page.set_compact(true);
            assert!(bar());
            page.selection().set_active(true);
            assert!(!bar() && page.selection().bar().is_revealed());
            page.selection().set_active(false);
            assert!(bar());
        });
    }

    #[test]
    fn a_narrow_window_shows_the_selection_buttons_without_words() {
        crate::testing::with_widgets(|| {
            let (_tmp, _state, page, _) = page_of(1);
            let imp = page.imp();
            assert!(!imp.selection_bar.compact());
            // Under 500 sp, then under 400 sp, where the page's own
            // breakpoint applies instead.
            imp.selection_narrow_on.set(true);
            page.update_selection_compact();
            assert!(imp.selection_bar.compact() && imp.selection_header.compact());
            imp.selection_narrow_on.set(false);
            page.set_compact(true);
            assert!(imp.selection_bar.compact() && imp.selection_header.compact());
            page.set_compact(false);
            assert!(!imp.selection_bar.compact());
        });
    }

    /// Every item shows whether its card is chosen.
    fn ticks_match(page: &DeckPage) {
        let chosen = page.selection().chosen_ids();
        let n = page.imp().cards.get().unwrap().n_items();
        for p in 0..n {
            let item = page.item(p).unwrap();
            let id = item.card().id.clone();
            assert_eq!(item.selected(), chosen.contains(&id), "card {p}");
        }
    }

    /// A row replaced while cards are chosen (here by the Undo of a card
    /// menu's Suspend) shows its card chosen: the controller follows the
    /// list, not the page.
    #[test]
    fn a_row_replaced_while_choosing_keeps_its_tick() {
        crate::testing::with_widgets(|| {
            let (_tmp, state, page, ids) = page_of(3);
            let window = presented(&page);
            page.activate_action("deck.suspend-card", Some(&ids[1].to_variant()))
                .unwrap();
            gtk::prelude::GtkWindowExt::set_focus(&window, Some(&*page.imp().card_list));
            assert!(press_ctrl_a(&page));
            assert_eq!(page.selection().chosen_ids().len(), 3);
            // The "Card suspended" toast's Undo.
            undo_suspend(&state, &ids[1], true, &page.downgrade(), None);
            assert!(!page.item(1).unwrap().card().suspended);
            ticks_match(&page);
            assert!(page.item(1).unwrap().selected());
            // A card that leaves the list behind the page's back is no
            // longer chosen.
            state.delete_card(&ids[0]).unwrap();
            page.reload();
            assert_eq!(page.selection().chosen_ids().len(), 2);
            assert_eq!(page.selection().header().title(), "2 Selected");
            ticks_match(&page);
            window.destroy();
        });
    }

    #[test]
    fn a_long_press_while_choosing_chooses_or_drops_the_card() {
        crate::testing::with_widgets(|| {
            let (_tmp, _state, page, ids) = page_of(2);
            let press = |id: &String| {
                page.activate_action("deck.select-card", Some(&id.to_variant()))
                    .unwrap();
            };
            press(&ids[0]);
            assert_eq!(page.selection().chosen_ids(), [ids[0].clone()]);
            press(&ids[1]);
            assert_eq!(page.selection().chosen_ids(), ids);
            press(&ids[0]);
            assert_eq!(page.selection().chosen_ids(), [ids[1].clone()]);
            assert!(page.selection().active(), "dropping a card keeps the mode");
        });
    }

    /// A page under another one does not take the keyboard focus when its
    /// list empties (here behind its back, then read again).
    #[test]
    fn a_page_under_another_does_not_take_the_focus() {
        crate::testing::with_widgets(|| {
            let (_tmp, state, page, ids) = page_of(1);
            let view = adw::NavigationView::new();
            let window = gtk::Window::builder().child(&view).build();
            window.present();
            view.push(&page);
            let top_button = gtk::Button::with_label("On top");
            let top = adw::NavigationPage::new(&top_button, "Top");
            view.push(&top);
            page.selection().set_active(true);
            page.selection().toggle(&ids[0]);
            top_button.grab_focus();
            state.delete_card(&ids[0]).unwrap();
            page.reload();
            assert!(!page.selection().active(), "the empty list ends the mode");
            assert_eq!(
                gtk::prelude::GtkWindowExt::focus(&window),
                Some(top_button.upcast())
            );
            window.destroy();
        });
    }

    /// Practice in the Deck Menu is offered while the deck has a card to
    /// practise, due or not, and pushes a practice of the deck (ADR 0070).
    #[test]
    fn the_deck_menu_offers_practice_while_a_card_can_be_practised() {
        crate::testing::with_widgets(|| {
            let (_tmp, state, page, ids) = page_of(1);
            let view = adw::NavigationView::new();
            let window = adw::Window::builder().content(&view).build();
            view.push(&page);
            window.present();
            assert!(crate::testing::menu_item_sensitive(
                &page.imp().deck_menu_button,
                "Practice"
            ));
            page.activate_action("deck.practice", None).unwrap();
            let practice = view.visible_page().and_downcast::<ReviewPage>().unwrap();
            assert!(practice.is_practice());
            view.pop();
            // Suspended, the only card cannot be practised.
            state.set_suspended(&ids, true).unwrap();
            page.update_study();
            assert!(!crate::testing::menu_item_sensitive(
                &page.imp().deck_menu_button,
                "Practice"
            ));
            window.destroy();
        });
    }
}
