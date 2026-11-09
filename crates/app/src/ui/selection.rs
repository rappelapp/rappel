//! Selection mode for a page that lists cards, shared by All Cards and a
//! deck's page so the two cannot drift (#104, ADR 0065; SPEC §4.6).
//!
//! The page owns its list (a `gio::ListStore` of a [`SelectableCard`] type)
//! and what fills it (a search and filters, or one deck). [`CardSelection`]
//! owns the rest: which cards are chosen ([`chosen::Selection`]), the
//! switch between the page's own header and [`SelectionHeader`] (the count
//! as the title, Select All, Cancel), the [`SelectionBar`] of actions, the
//! dialogs those actions ask with, the writes through `AppState`
//! ([`bulk`]), and the Undo toasts. The page answers what only it knows
//! through [`SelectionPage`].
//!
//! The actions are the page's `selection.*` group (a `GSimpleActionGroup`
//! inserted on the page), so the page's Select button, the shared header and
//! the shared bar all reach them: `choose`, `cancel`, `select-all`, `move`,
//! `add-tags`, `remove-tag`, `suspend`, `reset`, `delete`.
//!
//! Keys: Ctrl+A chooses every card listed, from anywhere on the page but a
//! text field (`ui::select_all`); Space chooses or drops the focused row
//! while choosing (a Capture controller on the list, so it comes before the
//! list view's own bindings: the item's `listitem.select` does nothing on a
//! list without a selection model). Escape is the page's, which asks
//! [`CardSelection::active`] first.

pub mod bulk;
pub mod chosen;
pub mod text;

use std::cell::{Cell, RefCell};
use std::marker::PhantomData;
use std::rc::{Rc, Weak};

use adw::prelude::*;
use gtk::{gio, glib};
use rappel_core::domain::{Deck, Tag};

pub use bulk::{Bulk, Done, perform};
use chosen::Selection;
use text::{ListKey, SpaceAction, SuspendChoice};

use crate::dialogs::{browser_dialogs, move_card};
use crate::i18n::gettext;
use crate::state::AppState;
use crate::ui::focus::index_after_removal;
use crate::ui::text::with_error;
use crate::ui::toast::{info_toast, log_failure, show_error, show_toast_from, undo_toast};
use crate::widgets::{CardRow, SelectionBar, SelectionHeader};
use crate::window::RappelWindow;

/// The name of the page's action group.
pub const GROUP: &str = "selection";

/// The actions on the chosen cards, enabled while one is chosen.
const ON_CHOSEN: [&str; 4] = ["add-tags", "remove-tag", "suspend", "delete"];

/// A list item that can be chosen: All Cards' `BrowseItem` and the deck
/// page's `CardObject`.
pub trait SelectableCard: IsA<glib::Object> {
    fn card_id(&self) -> String;
    fn card_suspended(&self) -> bool;
    /// No surviving review since it was created or last reset.
    fn card_is_new(&self) -> bool;
    /// The card's tags, for Remove Tag….
    fn card_tags(&self) -> Vec<Tag>;
    /// Shows the item chosen or not (its row's check button follows).
    fn set_chosen(&self, chosen: bool);
}

/// What selection mode asks of the page that lists the cards.
pub trait SelectionPage: IsA<adw::NavigationPage> + IsA<gtk::Widget> {
    /// Selection mode started (`true`) or ended: the rows and the page's
    /// own controls follow.
    fn selection_mode_changed(&self, active: bool);
    /// Cards were written: read the list again, and tell what else shows
    /// them.
    fn cards_changed(&self);
    /// Nothing was written, but the rows may be stale: read them again.
    fn reread_cards(&self);
    /// Whether the list is shown (not an empty state).
    fn list_shown(&self) -> bool;
    /// No row is left: focus the empty state's button.
    fn focus_empty_state(&self);
    /// Whether another deck exists to move the cards to.
    fn can_move_cards(&self) -> bool;
}

/// The page's widgets selection mode works with.
#[derive(Debug)]
pub struct Parts {
    /// The page's own header, shown outside selection mode.
    pub own_header: gtk::Widget,
    /// The page's Select button (`selection.choose`), in its own header.
    pub select_button: gtk::Button,
    /// Where Select goes when the page's header has no room for it (the
    /// deck page's bottom bar on a phone), if the page has such a place.
    /// The focus goes there when the header's button is not showing.
    pub narrow_select_button: Option<gtk::Button>,
    pub header: SelectionHeader,
    pub bar: SelectionBar,
    pub list: gtk::ListView,
    /// The list's items, in the order shown (the list view's model under
    /// its selection model): positions here are the rows' positions.
    pub items: gio::ListModel,
    /// The deck whose page this is: Move… does not offer it.
    pub own_deck: Option<String>,
}

struct Inner<P: glib::object::ObjectType, T> {
    page: glib::WeakRef<P>,
    state: AppState,
    parts: Parts,
    actions: gio::SimpleActionGroup,
    selection: RefCell<Selection>,
    /// The mode the page shows; [`CardSelection::update`] brings it in
    /// line with the selection.
    shown: Cell<bool>,
    /// The Undo of the last toast, for the tests to press.
    #[cfg(test)]
    last_undo: RefCell<Option<Rc<dyn Fn()>>>,
    item: PhantomData<T>,
}

/// Where the focus goes after an action: the card it started with, or the
/// row now at its place (see [`CardSelection::refocus`]).
type FocusTarget = (Option<u32>, Option<String>);

/// Selection mode of one page (see the module's text). Cheap to clone.
pub struct CardSelection<P: glib::object::ObjectType, T>(Rc<Inner<P, T>>);

impl<P: glib::object::ObjectType, T> Clone for CardSelection<P, T> {
    fn clone(&self) -> Self {
        Self(Rc::clone(&self.0))
    }
}

impl<P: glib::object::ObjectType, T> std::fmt::Debug for CardSelection<P, T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CardSelection")
            .field("active", &self.0.selection.borrow().active())
            .field("chosen", &self.0.selection.borrow().len())
            .finish_non_exhaustive()
    }
}

impl<P: SelectionPage, T: SelectableCard> CardSelection<P, T> {
    /// Selection mode for `page`, with its actions, Ctrl+A and Space.
    pub fn new(page: &P, state: AppState, parts: Parts) -> Self {
        let inner = Rc::new(Inner {
            page: page.downgrade(),
            state,
            parts,
            actions: gio::SimpleActionGroup::new(),
            selection: RefCell::default(),
            shown: Cell::new(false),
            #[cfg(test)]
            last_undo: RefCell::default(),
            item: PhantomData,
        });
        let selection = Self(inner);
        selection.add_actions();
        page.insert_action_group(GROUP, Some(&selection.0.actions));
        // Ctrl+A (#106): see `ui::select_all` for the GTK sources.
        let weak = Rc::downgrade(&selection.0);
        let _ = crate::ui::select_all::install(page, move || {
            if let Some(selection) = Self::upgrade(&weak) {
                selection.select_all();
            }
        });
        selection.add_space();
        selection.follow_items();
        selection.update();
        selection
    }

    fn upgrade(weak: &Weak<Inner<P, T>>) -> Option<Self> {
        weak.upgrade().map(Self)
    }

    fn add_actions(&self) {
        let add = |name: &str, f: fn(&Self)| {
            let action = gio::SimpleAction::new(name, None);
            let weak = Rc::downgrade(&self.0);
            action.connect_activate(move |_, _| {
                if let Some(selection) = Self::upgrade(&weak) {
                    f(&selection);
                }
            });
            self.0.actions.add_action(&action);
        };
        add("choose", Self::start);
        add("cancel", |s| s.set_active(false));
        add("select-all", Self::select_all);
        add("move", Self::move_selected);
        add("add-tags", |s| {
            let _ = s.add_tags();
        });
        add("remove-tag", Self::remove_tag);
        add("suspend", Self::suspend_selected);
        add("reset", |s| s.run_selected(Bulk::Reset));
        add("delete", |s| s.run_selected(Bulk::Delete));
    }

    fn set_enabled(&self, name: &str, enabled: bool) {
        if let Some(action) = self
            .0
            .actions
            .lookup_action(name)
            .and_downcast::<gio::SimpleAction>()
        {
            action.set_enabled(enabled);
        }
    }

    /// Space on the list: capture phase, so it comes before the list
    /// view's own bindings.
    fn add_space(&self) {
        let keys = gtk::EventControllerKey::new();
        keys.set_propagation_phase(gtk::PropagationPhase::Capture);
        let weak = Rc::downgrade(&self.0);
        keys.connect_key_pressed(move |_, key, _, modifiers| {
            let Some(selection) = Self::upgrade(&weak) else {
                return glib::Propagation::Proceed;
            };
            match text::list_key(key, modifiers) {
                Some(ListKey::Space) => {
                    match text::space_action(selection.active(), selection.row_has_focus()) {
                        SpaceAction::Toggle => {
                            if let Some(id) = selection.focused_card() {
                                selection.toggle(&id);
                            }
                            glib::Propagation::Stop
                        }
                        SpaceAction::Pass => glib::Propagation::Proceed,
                    }
                }
                None => glib::Propagation::Proceed,
            }
        });
        self.0.parts.list.add_controller(keys);
    }

    // --- the list ---------------------------------------------------------

    fn item(&self, position: u32) -> Option<T> {
        self.0.parts.items.item(position).and_downcast()
    }

    fn items(&self) -> impl Iterator<Item = T> + '_ {
        (0..self.0.parts.items.n_items()).filter_map(|p| self.item(p))
    }

    /// Where card `id` is in the list. A linear scan, only on a user action.
    fn position(&self, id: &str) -> Option<u32> {
        (0..self.0.parts.items.n_items()).find(|&p| self.item(p).is_some_and(|i| i.card_id() == id))
    }

    fn ids(&self) -> Vec<String> {
        self.items().map(|i| i.card_id()).collect()
    }

    /// Whether a row itself has the keyboard focus (not a button in it).
    fn row_has_focus(&self) -> bool {
        let list = &self.0.parts.list;
        let focused = list.root().and_then(|root| root.focus());
        focused.is_some() && focused == list.focus_child()
    }

    /// The card whose row has the keyboard focus.
    fn focused_card(&self) -> Option<String> {
        let row = self
            .0
            .parts
            .list
            .focus_child()?
            .first_child()
            .and_downcast::<CardRow>()?;
        let id = row.card_id();
        (!id.is_empty()).then_some(id)
    }

    /// Scrolls to the card at `position` and gives it the keyboard focus.
    fn focus(&self, position: u32) {
        crate::ui::focus::focus_row(&self.0.parts.list, position);
    }

    /// Keeps the items in line with the choice whatever changes the list,
    /// so no page can forget it: an item put in the list (read again, or a
    /// row replaced after a suspend, a reset, an edit or an Undo) shows
    /// whether its card is chosen, and the cards that left the list are no
    /// longer chosen. `GListModel::items-changed` is emitted after the
    /// change, with the position and how many items left and came
    /// (`GListModel` docs), so the new items are in place.
    fn follow_items(&self) {
        let weak = Rc::downgrade(&self.0);
        self.0
            .parts
            .items
            .connect_items_changed(move |items, position, removed, added| {
                let Some(selection) = Self::upgrade(&weak) else {
                    return;
                };
                // Nothing chosen (always, outside selection mode): nothing
                // to drop, and no O(n) walk of the list on every change
                // (a deck page's refresh, an edit).
                if removed > 0 && selection.0.selection.borrow().len() > 0 {
                    let ids = selection.ids();
                    selection
                        .0
                        .selection
                        .borrow_mut()
                        .retain(ids.iter().map(String::as_str));
                }
                for p in position..position + added {
                    if let Some(item) = items.item(p).and_downcast::<T>() {
                        item.set_chosen(selection.is_chosen(&item.card_id()));
                    }
                }
            });
    }

    /// Whether the page is the one the navigation view shows (or is in no
    /// navigation view): only then may it move the keyboard focus, which a
    /// page under another one would take from it.
    fn page_shown(&self) -> bool {
        let Some(page) = self.0.page.upgrade() else {
            return false;
        };
        page.ancestor(adw::NavigationView::static_type())
            .and_downcast::<adw::NavigationView>()
            .is_none_or(|view| view.visible_page().as_ref() == Some(page.upcast_ref()))
    }

    /// After the list changed: Select is enabled while there is a card to
    /// choose, and a list that became empty ends selection mode, with the
    /// focus on the empty state's button.
    pub fn list_changed(&self) {
        let empty = self.0.parts.items.n_items() == 0;
        self.set_enabled("choose", !empty);
        if empty && self.active() {
            self.set_active(false);
            if self.page_shown()
                && let Some(page) = self.0.page.upgrade()
            {
                page.focus_empty_state();
            }
        } else {
            self.update();
        }
    }

    // --- choosing ---------------------------------------------------------

    /// Whether the page is in selection mode.
    pub fn active(&self) -> bool {
        self.0.selection.borrow().active()
    }

    /// Whether card `id` is chosen.
    pub fn is_chosen(&self, id: &str) -> bool {
        self.0.selection.borrow().contains(id)
    }

    /// The chosen cards' ids, in list order.
    pub fn chosen_ids(&self) -> Vec<String> {
        let ids = self.ids();
        self.0
            .selection
            .borrow()
            .ids_in(ids.iter().map(String::as_str))
    }

    /// The listed items that are chosen, in list order.
    fn chosen_items(&self) -> Vec<T> {
        let selection = self.0.selection.borrow();
        self.items()
            .filter(|i| selection.contains(&i.card_id()))
            .collect()
    }

    /// Enters or leaves selection mode. Leaving forgets the choice.
    pub fn set_active(&self, on: bool) {
        if on {
            self.0.selection.borrow_mut().enter();
        } else {
            self.0.selection.borrow_mut().leave();
            for item in self.items() {
                item.set_chosen(false);
            }
        }
        self.update();
    }

    /// The Select button: start choosing. The button hides, so the
    /// keyboard goes to the list (or stays where it was if it is empty).
    pub fn start(&self) {
        self.set_active(true);
        if let Some(page) = self.0.page.upgrade()
            && page.list_shown()
        {
            self.0.parts.list.grab_focus();
        }
    }

    /// Chooses every card listed, entering selection mode if needed
    /// (Ctrl+A, Select All; #106). Nothing to choose, nothing done.
    pub fn select_all(&self) {
        let ids = self.ids();
        if ids.is_empty() {
            return;
        }
        self.0
            .selection
            .borrow_mut()
            .select_all(ids.iter().map(String::as_str));
        for item in self.items() {
            item.set_chosen(true);
        }
        self.update();
    }

    /// Chooses card `id` or, if chosen, drops it.
    pub fn toggle(&self, id: &str) {
        let Some(item) = self.position(id).and_then(|p| self.item(p)) else {
            return;
        };
        let chosen = self.0.selection.borrow_mut().toggle(id);
        item.set_chosen(chosen);
        self.update();
    }

    /// A long press on a row: start choosing with this card, or, while
    /// choosing, choose or drop it (as a tap does).
    pub fn press(&self, id: &str) {
        if !self.active() {
            self.set_active(true);
        }
        self.toggle(id);
    }

    /// A row was activated (a click, Enter): in selection mode it is chosen
    /// or dropped, and `true` is returned; otherwise the page opens it.
    pub fn activate(&self, position: u32) -> bool {
        if !self.active() {
            return false;
        }
        if let Some(item) = self.item(position) {
            self.toggle(&item.card_id());
        }
        true
    }

    /// Which way Suspend goes for the chosen cards.
    pub fn suspend_choice(&self) -> SuspendChoice {
        text::suspend_choice(
            self.chosen_items()
                .iter()
                .map(SelectableCard::card_suspended),
        )
    }

    /// Brings the header, the action bar and the actions in line with the
    /// selection.
    pub fn update(&self) {
        let inner = &self.0;
        let Some(page) = inner.page.upgrade() else {
            return;
        };
        let parts = &inner.parts;
        let (active, n) = {
            let selection = inner.selection.borrow();
            (selection.active(), selection.len())
        };
        if inner.shown.get() != active {
            // The window's focus widget, not `has_focus`, which is false
            // while the window is not the active one.
            let focus = page.root().and_then(|root| root.focus());
            let within = |widget: &gtk::Widget| {
                focus
                    .as_ref()
                    .is_some_and(|f| f == widget || f.is_ancestor(widget))
            };
            // A button of the mode about to hide must not keep the keyboard:
            // it goes to the Select button that takes its place.
            let in_mode = within(parts.header.upcast_ref()) || within(parts.bar.upcast_ref());
            // Entering the mode hides the page's own header the same way
            // (Ctrl+A does it from anywhere): the keyboard goes to the list,
            // or to Cancel when there is no list to go to.
            let in_own_header = within(&parts.own_header);
            inner.shown.set(active);
            // The header of selection mode (HIG): how many are chosen,
            // Select All and Cancel; Back hides and Escape does not go back
            // (`AdwNavigationPage:can-pop` docs), so Escape leaves the mode.
            parts.own_header.set_visible(!active);
            parts.select_button.set_visible(!active);
            parts.header.set_visible(active);
            page.set_can_pop(!active);
            parts.bar.set_revealed(active);
            page.selection_mode_changed(active);
            if in_mode && !active {
                // `is_visible` looks at the ancestors too: the button of a
                // header whose slot a breakpoint hides cannot take it.
                match &parts.narrow_select_button {
                    Some(narrow) if !parts.select_button.is_visible() => narrow.grab_focus(),
                    _ => parts.select_button.grab_focus(),
                };
            }
            if in_own_header && active {
                if page.list_shown() {
                    parts.list.grab_focus();
                } else {
                    parts.header.cancel_button().grab_focus();
                }
            }
        }
        parts
            .header
            .set_titles(&text::selection_title(n), &text::selection_title_tight(n));
        for name in ON_CHOSEN {
            self.set_enabled(name, n > 0);
        }
        self.set_enabled("move", n > 0 && page.can_move_cards());
        // Nothing chosen, no items to look at: this runs on every change
        // of the list, so it must not walk it then (O(n) per change).
        let chosen = if n == 0 {
            Vec::new()
        } else {
            self.chosen_items()
        };
        self.set_enabled(
            "reset",
            text::reset_offered(chosen.iter().map(SelectableCard::card_is_new)),
        );
        parts.bar.set_tags_sensitive(n > 0);
        parts.bar.show_suspend(text::suspend_choice(
            chosen.iter().map(SelectableCard::card_suspended),
        ));
    }

    // --- actions on the chosen cards --------------------------------------

    fn run_selected(&self, action: Bulk) {
        self.run(&self.chosen_ids(), action);
    }

    fn suspend_selected(&self) {
        let suspend = self.suspend_choice() == SuspendChoice::Suspend;
        self.run_selected(Bulk::Suspend(suspend));
    }

    /// Move…: asks for a deck (not the page's own) and moves the chosen
    /// cards there.
    fn move_selected(&self) {
        let ids = self.chosen_ids();
        let Some(page) = self.0.page.upgrade() else {
            return;
        };
        if ids.is_empty() {
            return;
        }
        let decks = match self.0.state.decks() {
            Ok(decks) => decks,
            Err(err) => {
                // Translators: a toast: what is before ":" is its title, the rest is behind Details. {error} is why, in English.
                self.error_toast(&with_error(
                    &gettext("The decks could not be read: {error}"),
                    &err,
                ));
                return;
            }
        };
        let decks = move_targets(decks, self.0.parts.own_deck.as_deref());
        if decks.is_empty() {
            show_toast_from(&page, info_toast(&move_card::no_other_deck_text()));
            return;
        }
        let selection = self.clone();
        glib::spawn_future_local(async move {
            let Some(page) = selection.0.page.upgrade() else {
                return;
            };
            let names: Vec<String> = decks.iter().map(|d| d.name.clone()).collect();
            let body = text::move_body(ids.len());
            let (refocus, closed) = selection.refocus_when_closed();
            let chosen = browser_dialogs::choose_closing(
                &page,
                &gettext("Move Cards"),
                &body,
                &gettext("_Move"),
                &names,
                closed,
            )
            .await;
            if let Some(deck) = chosen.and_then(|i| decks.get(i)) {
                let target = selection.run(
                    &ids,
                    Bulk::Move {
                        deck_id: deck.id.clone(),
                    },
                );
                refocus.replace(target);
            }
        });
    }

    /// Asks for tags in the card editor's tag entry, with the tags in use
    /// as suggestions (ADR 0051, 0052), and puts them on the chosen cards.
    /// Returns the dialog (tests answer it).
    pub fn add_tags(&self) -> Option<adw::AlertDialog> {
        let ids = self.chosen_ids();
        if ids.is_empty() {
            return None;
        }
        let page = self.0.page.upgrade()?;
        let all = self.0.state.used_tags().unwrap_or_else(|err| {
            tracing::warn!("the tags could not be read for suggestions: {err}");
            Vec::new()
        });
        let body = text::add_tags_body(ids.len());
        let (refocus, closed) = self.refocus_when_closed();
        let weak = Rc::downgrade(&self.0);
        let dialog = browser_dialogs::add_tags(&page, &body, all, move |names| {
            if let Some(selection) = Self::upgrade(&weak) {
                let target = selection.run(&ids, Bulk::AddTags(names));
                refocus.replace(target);
            }
        })?;
        dialog.connect_closed(move |_| closed());
        Some(dialog)
    }

    /// Remove Tag…: asks which of the chosen cards' tags to take off them.
    fn remove_tag(&self) {
        let ids = self.chosen_ids();
        if ids.is_empty() {
            return;
        }
        let Some(page) = self.0.page.upgrade() else {
            return;
        };
        // Only the tags the chosen cards have.
        let mut tags: Vec<Tag> = Vec::new();
        for item in self.chosen_items() {
            for tag in item.card_tags() {
                if !tags.iter().any(|t| t.id == tag.id) {
                    tags.push(tag);
                }
            }
        }
        if tags.is_empty() {
            show_toast_from(&page, info_toast(&text::no_tags_text()));
            return;
        }
        let selection = self.clone();
        glib::spawn_future_local(async move {
            let Some(page) = selection.0.page.upgrade() else {
                return;
            };
            let names: Vec<String> = tags.iter().map(|t| t.name.clone()).collect();
            let body = text::remove_tag_body(ids.len());
            let (refocus, closed) = selection.refocus_when_closed();
            let chosen = browser_dialogs::choose_closing(
                &page,
                &gettext("Remove Tag"),
                &body,
                &gettext("_Remove"),
                &names,
                closed,
            )
            .await;
            if let Some(tag) = chosen.and_then(|i| tags.get(i)) {
                let target = selection.run(&ids, Bulk::RemoveTag(tag.clone()));
                refocus.replace(target);
            }
        });
    }

    /// A dialog gives the keyboard focus back to the widget that had it
    /// (a button of the action bar, hidden by then) when it closes, after
    /// its answer (`browser_dialogs::choose_closing`): the focus an action
    /// set is set again then. Returns where to note that focus, and what to
    /// call when the dialog has closed.
    fn refocus_when_closed(&self) -> (Rc<RefCell<Option<FocusTarget>>>, impl Fn() + 'static) {
        let target: Rc<RefCell<Option<FocusTarget>>> = Rc::default();
        let weak = Rc::downgrade(&self.0);
        let noted = Rc::clone(&target);
        let closed = move || {
            if let (Some(selection), Some((first, id))) = (Self::upgrade(&weak), noted.take()) {
                selection.refocus(first, id.as_deref());
            }
        };
        (target, closed)
    }

    /// Does `action` on `ids` at once and offers Undo in a toast. Returns
    /// where the focus went if something was written.
    pub fn run(&self, ids: &[String], action: Bulk) -> Option<FocusTarget> {
        let page = self.0.page.upgrade()?;
        let state = self.0.state.clone();
        let first = ids.first().and_then(|id| self.position(id));
        let first_id = ids.first().cloned();
        match perform(&state, ids, action, rappel_core::time::now_ms()) {
            Ok(Ok(done)) => {
                self.set_active(false);
                page.cards_changed();
                self.refocus(first, first_id.as_deref());
                self.offer_undo(&page, state, done);
                Some((first, first_id))
            }
            Ok(Err(notice)) => {
                // Nothing was written, but the rows may be stale (a card
                // reset elsewhere still offering Reset Progress).
                page.reread_cards();
                show_toast_from(&page, info_toast(&notice));
                None
            }
            Err(rappel_core::Error::CardNotFound(_) | rappel_core::Error::DeckNotFound(_)) => {
                // Another window or an import removed a card or a deck: the
                // list shown is old. Nothing was written (the write is all or
                // nothing).
                page.cards_changed();
                show_toast_from(&page, info_toast(&text::stale_text()));
                None
            }
            Err(err) => {
                // Translators: a toast: what is before ":" is its title, the rest is behind Details. {error} is why, in English.
                self.error_toast(&with_error(
                    &gettext("The cards could not be changed: {error}"),
                    &err,
                ));
                None
            }
        }
    }

    /// After an action the focus goes to the card it started with, or, if
    /// that left the list, to the row now at its place, else the last one,
    /// else the empty state's button (`ui::focus::index_after_removal`).
    ///
    /// Always set, because the action bar was hiding the focused button.
    pub fn refocus(&self, first: Option<u32>, first_id: Option<&str>) {
        let (Some(position), Some(id)) = (first, first_id) else {
            return;
        };
        if !self.page_shown() {
            return;
        }
        if let Some(still) = self.position(id) {
            self.focus(still);
            return;
        }
        match index_after_removal(position, self.0.parts.items.n_items()) {
            Some(next) => self.focus(next),
            None => {
                if let Some(page) = self.0.page.upgrade() {
                    page.focus_empty_state();
                }
            }
        }
    }

    /// The toast with Undo. It must work after leaving the page, so the
    /// closure keeps the state and weak references, not the page.
    fn offer_undo(&self, page: &P, state: AppState, done: Done) {
        let weak = Rc::downgrade(&self.0);
        let window = page
            .root()
            .and_downcast::<RappelWindow>()
            .map(|w| w.downgrade());
        let note = done.note.clone();
        let title = done.text.clone();
        let on_undo: Rc<dyn Fn()> = Rc::new(move || undo(&state, &done, &weak, window.as_ref()));
        #[cfg(test)]
        self.0.last_undo.replace(Some(Rc::clone(&on_undo)));
        let toast = undo_toast(&title, move || on_undo());
        show_toast_from(page, toast);
        // Queued after the Undo toast, which has the high priority.
        if let Some(note) = note {
            show_toast_from(page, info_toast(&note));
        }
    }

    fn error_toast(&self, message: &str) {
        match self.0.page.upgrade() {
            Some(page) => show_error(&page, "a card operation", message),
            None => log_failure("a card operation", message),
        }
    }

    /// Presses Undo on the last toast, as the user would, for the tests.
    #[cfg(test)]
    pub fn undo_last(&self) {
        let on_undo = self.0.last_undo.borrow_mut().take();
        if let Some(on_undo) = on_undo {
            on_undo();
        }
    }

    /// The header, for the tests.
    #[cfg(test)]
    pub fn header(&self) -> &SelectionHeader {
        &self.0.parts.header
    }

    /// The action bar, for the tests.
    #[cfg(test)]
    pub fn bar(&self) -> &SelectionBar {
        &self.0.parts.bar
    }
}

/// The decks Move… offers: every deck but the page's own (`own_deck`, on a
/// deck's page; `None` on All Cards, whose cards come from every deck).
fn move_targets(decks: Vec<Deck>, own_deck: Option<&str>) -> Vec<Deck> {
    match own_deck {
        Some(own) => move_card::targets(&decks, own)
            .into_iter()
            .cloned()
            .collect(),
        // Archived decks are not offered (ADR 0068); `decks` has none,
        // but the rule is kept where the targets are chosen.
        None => decks
            .into_iter()
            .filter(|deck| !deck.is_archived())
            .collect(),
    }
}

/// The Undo of a bulk action. What it could not restore is said in a note.
fn undo<P: SelectionPage, T>(
    state: &AppState,
    done: &Done,
    selection: &Weak<Inner<P, T>>,
    window: Option<&glib::WeakRef<RappelWindow>>,
) {
    let window = window.and_then(glib::WeakRef::upgrade);
    match (done.undo)(state) {
        Ok(restored) => {
            // On the page if it is still shown, else through the window
            // (the deck pages, All Cards and the deck list read it).
            let page = selection
                .upgrade()
                .and_then(|inner| inner.page.upgrade())
                .filter(|p| p.root().is_some());
            match page {
                Some(page) => {
                    page.cards_changed();
                    if let Some(note) = text::partial_undo_text(restored, done.total) {
                        show_toast_from(&page, info_toast(&note));
                    }
                }
                None => {
                    if let Some(window) = &window {
                        window.data_changed();
                        if let Some(note) = text::partial_undo_text(restored, done.total) {
                            show_toast_from(window, info_toast(&note));
                        }
                    }
                }
            }
        }
        Err(err) => {
            // Translators: a toast: what is before ":" is its title, the rest is behind Details. {error} is why, in English.
            let message = with_error(&gettext("The change could not be undone: {error}"), &err);
            match window {
                Some(window) => show_error(&window, "a card operation", &message),
                None => log_failure("a card operation", &message),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn deck(id: &str) -> Deck {
        Deck {
            id: id.to_owned(),
            name: id.to_owned(),
            created_at: 0,
            updated_at: 0,
            archived_at: None,
        }
    }

    #[test]
    fn move_offers_every_deck_but_the_pages_own() {
        let ids = |decks: Vec<Deck>| -> Vec<String> { decks.into_iter().map(|d| d.id).collect() };
        let decks = vec![deck("a"), deck("b"), deck("c")];
        assert_eq!(ids(move_targets(decks.clone(), Some("b"))), ["a", "c"]);
        // All Cards: every deck, its cards come from all of them.
        assert_eq!(ids(move_targets(decks, None)), ["a", "b", "c"]);
        // A deck's page with no other deck: nothing to offer.
        assert_eq!(
            ids(move_targets(vec![deck("a")], Some("a"))),
            Vec::<String>::new()
        );
    }
}
