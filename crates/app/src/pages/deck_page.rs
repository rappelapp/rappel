//! One deck's page: its cards in a `GtkListView`, or an empty state, with
//! add, edit and delete (undone from a toast). Layout in
//! `data/deck_page.blp` (ADR 0013).
//!
//! The subtitle and the Study button follow what there is to study. A
//! one-shot timer re-checks it when the next card becomes due or the next
//! study day begins, so they change while the page is open.
//!
//! The list model is a `gio::ListStore` of [`CardObject`] in creation
//! order. Changes update it item by item instead of rebuilding it, so a
//! deck of thousands of cards keeps its scroll position and stays fast.

use std::cmp::Ordering;
use std::time::Duration;

use adw::prelude::*;
use adw::subclass::prelude::*;
use cards_core::domain::{Card, Deck};
use cards_core::scheduling::session::StudyInput;
use gtk::{gio, glib};

use crate::clock::{local_day_start_ms, local_next_day_start_ms};
use crate::dialogs::card_editor::{CardEditor, Saved};
use crate::pages::ReviewPage;
use crate::state::AppState;
use crate::ui::focus::index_after_removal;
use crate::ui::toast::{log_failure, persistent_toast, show_error, show_toast_from, undo_toast};
use crate::ui::{init_once, nav};
use crate::widgets::{CardObject, CardRow};
use crate::window::CardsWindow;

mod imp {
    use std::cell::{Cell, OnceCell, RefCell};

    use adw::prelude::*;
    use adw::subclass::prelude::*;
    use cards_core::domain::Deck;
    use gtk::{gio, glib};

    use crate::state::AppState;
    use crate::ui::rows;

    #[derive(Debug, Default, gtk::CompositeTemplate)]
    #[template(resource = "/io/github/rsmirke/Cards/deck_page.ui")]
    pub struct DeckPage {
        #[template_child]
        pub window_title: TemplateChild<adw::WindowTitle>,
        #[template_child]
        pub content_stack: TemplateChild<gtk::Stack>,
        #[template_child]
        pub card_list: TemplateChild<gtk::ListView>,
        #[template_child]
        pub empty_add_button: TemplateChild<gtk::Button>,
        pub state: OnceCell<AppState>,
        pub deck: OnceCell<Deck>,
        /// The cards, oldest first.
        pub cards: OnceCell<gio::ListStore>,
        /// An editor is open; a second one (Ctrl+N, a click) is not opened.
        pub editing: Cell<bool>,
        /// The card to focus when the editor closes: the one last saved.
        pub focus_after_edit: RefCell<Option<String>>,
        /// The pending re-check of what there is to study. Taken out by its
        /// own callback, so a stored id always names a live source.
        pub study_timer: RefCell<Option<glib::SourceId>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for DeckPage {
        const NAME: &'static str = "CardsDeckPage";
        type Type = super::DeckPage;
        type ParentType = adw::NavigationPage;

        fn class_init(klass: &mut Self::Class) {
            klass.bind_template();
            klass.install_action("deck.new-card", None, |page, _, _| {
                page.new_card();
            });
            klass.install_action("deck.study", None, |page, _, _| {
                page.study();
            });
            // Activated by the card rows' menus with the card id.
            rows::install_id_action(klass, "deck.edit-card", super::DeckPage::edit);
            rows::install_id_action(klass, "deck.delete-card", super::DeckPage::delete);
        }

        fn instance_init(obj: &glib::subclass::InitializingObject<Self>) {
            obj.init_template();
        }
    }

    impl ObjectImpl for DeckPage {
        fn constructed(&self) {
            self.parent_constructed();
            // Back from studying: what is left to study changed.
            self.obj().connect_shown(super::DeckPage::update_study);
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
    pub fn new(state: AppState, deck: Deck) -> cards_core::Result<Self> {
        let page: Self = glib::Object::new();
        let imp = page.imp();
        let cards = state.list_cards(&deck.id)?;
        // Deck names are untrusted; NavigationPage:title is plain text.
        page.set_title(&deck.name);
        imp.window_title.set_title(&deck.name);

        let store = gio::ListStore::new::<CardObject>();
        let items: Vec<CardObject> = cards.into_iter().map(CardObject::new).collect();
        store.extend_from_slice(&items);
        store.connect_items_changed(glib::clone!(
            #[weak]
            page,
            move |_, _, _, _| {
                page.update_empty_state();
                page.update_study();
            }
        ));
        imp.card_list
            .set_model(Some(&gtk::NoSelection::new(Some(store.clone()))));
        imp.card_list.set_factory(Some(&row_factory()));
        imp.card_list.connect_activate(glib::clone!(
            #[weak]
            page,
            move |_, position| {
                if let Some(item) = page.item(position) {
                    page.open_editor(Some(&item.card()));
                }
            }
        ));

        init_once(&imp.state, state);
        init_once(&imp.deck, deck);
        init_once(&imp.cards, store);
        page.update_empty_state();
        page.update_study();
        Ok(page)
    }

    /// Recomputes what there is to study: the subtitle and whether Study
    /// is enabled.
    fn update_study(&self) {
        let imp = self.imp();
        let (Some(state), Some(deck)) = (imp.state.get(), imp.deck.get()) else {
            return;
        };
        let now = cards_core::time::now_ms();
        match state.study_input(&deck.id, now, local_day_start_ms(now)) {
            Ok(input) => {
                imp.window_title.set_subtitle(&study_subtitle(&input));
                self.action_set_enabled("deck.study", !input.is_empty());
                match state.next_study_change(&deck.id, now) {
                    Ok(next) => self.arm_study_timer(timer_delay_ms(
                        now,
                        next,
                        local_next_day_start_ms(now),
                    )),
                    // The subtitle is right now; it only will not update by
                    // itself until the next change on this page.
                    Err(err) => tracing::warn!("cannot read the next due time: {err}"),
                }
            }
            Err(err) => {
                imp.window_title.set_subtitle("");
                self.action_set_enabled("deck.study", false);
                self.error_toast(&format!("What to study could not be read: {err}"));
            }
        }
    }

    /// Re-checks what there is to study after `delay_ms`, replacing any
    /// pending re-check. Holds the page weakly: a closed page ends it.
    fn arm_study_timer(&self, delay_ms: u64) {
        let imp = self.imp();
        if let Some(pending) = imp.study_timer.take() {
            pending.remove();
        }
        let page = self.downgrade();
        let id = glib::timeout_add_local_once(Duration::from_millis(delay_ms), move || {
            if let Some(page) = page.upgrade() {
                // This source is ending; forget it before re-arming.
                page.imp().study_timer.take();
                page.update_study();
            }
        });
        imp.study_timer.replace(Some(id));
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
        // A double click or key repeat must not push two sessions.
        if view.visible_page().is_some_and(|p| p.is::<ReviewPage>()) {
            return;
        }
        // Cards saved from the session's editor update this list too.
        let on_saved = glib::clone!(
            #[weak(rename_to = deck_page)]
            self,
            move |saved| {
                deck_page.show_saved(saved);
            }
        );
        match ReviewPage::new(state.clone(), deck.clone(), on_saved) {
            Ok(page) => {
                self.action_set_enabled("deck.study", false);
                view.push(&page);
            }
            Err(err) => self.error_toast(&format!("The session could not start: {err}")),
        }
    }

    /// Opens the editor for a new card in this deck.
    pub fn new_card(&self) {
        self.open_editor(None);
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
        if imp.editing.replace(true) {
            return;
        }
        let editor = CardEditor::new(
            state.clone(),
            &deck.id,
            card,
            glib::clone!(
                #[weak(rename_to = page)]
                self,
                move |saved| page.saved(saved)
            ),
        );
        editor.connect_closed(glib::clone!(
            #[weak(rename_to = page)]
            self,
            move |_| {
                let imp = page.imp();
                imp.editing.set(false);
                if let Some(id) = imp.focus_after_edit.take()
                    && let Some(position) = page.position(&id)
                {
                    page.focus(position);
                }
            }
        ));
        editor.show(self);
    }

    /// Puts a card saved by this page's editor into the list and marks it
    /// to take the focus once the editor closes.
    fn saved(&self, saved: Saved) {
        if let Some(id) = self.show_saved(saved) {
            self.imp().focus_after_edit.replace(Some(id));
        }
    }

    /// Puts a saved card into the list, wherever the save came from (the
    /// review page's editor too). Returns its id.
    fn show_saved(&self, saved: Saved) -> Option<String> {
        let store = self.imp().cards.get()?;
        let card = match saved {
            Saved::Added(card) => {
                insert_in_order(store, &CardObject::new(card.clone()));
                card
            }
            Saved::Edited(card) => {
                match self.position(&card.id) {
                    Some(position) => store.splice(position, 1, &[CardObject::new(card.clone())]),
                    None => {
                        insert_in_order(store, &CardObject::new(card.clone()));
                    }
                }
                card
            }
        };
        Some(card.id)
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
            self.error_toast(&format!("The card could not be deleted: {err}"));
            return;
        }
        store.remove(position);
        match index_after_removal(position, store.n_items()) {
            Some(next) => self.focus(next),
            None => {
                imp.empty_add_button.grab_focus();
            }
        }

        // Undo must work after going back to the deck list too, when this
        // page may be gone: the closure keeps the state, not the page.
        let id = id.to_owned();
        let state = state.clone();
        let page = self.downgrade();
        let window = self
            .root()
            .and_downcast::<CardsWindow>()
            .map(|w| w.downgrade());
        // The front is not in the title: it can be long or many lines.
        let toast = undo_toast("Card deleted", move || {
            undo_card_delete(&state, &id, &page, window.as_ref());
        });
        show_toast_from(self, toast);
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
        let position = match self.position(&card.id) {
            Some(position) => position,
            None => insert_in_order(store, &CardObject::new(card.clone())),
        };
        self.focus(position);
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
        match state.list_cards(&deck.id) {
            Ok(cards) => {
                let items: Vec<CardObject> = cards.into_iter().map(CardObject::new).collect();
                store.splice(0, store.n_items(), &items);
            }
            Err(err) => self.error_toast(&format!("The cards could not be read: {err}")),
        }
    }

    /// Scrolls to the card at `position` and gives it the keyboard focus.
    fn focus(&self, position: u32) {
        self.imp()
            .card_list
            .scroll_to(position, gtk::ListScrollFlags::FOCUS, None);
    }

    fn update_empty_state(&self) {
        let imp = self.imp();
        let empty = imp.cards.get().is_none_or(|s| s.n_items() == 0);
        imp.content_stack
            .set_visible_child_name(if empty { "empty" } else { "list" });
    }

    fn item(&self, position: u32) -> Option<CardObject> {
        self.imp().cards.get()?.item(position).and_downcast()
    }

    /// Where card `id` is in the list. A linear scan: a few microseconds
    /// for thousands of cards, and only on a user action.
    fn position(&self, id: &str) -> Option<u32> {
        let store = self.imp().cards.get()?;
        (0..store.n_items()).find(|&position| {
            store
                .item(position)
                .and_downcast::<CardObject>()
                .is_some_and(|c| c.card().id == id)
        })
    }

    fn error_toast(&self, message: &str) {
        show_error(self, "a card operation", message);
    }
}

/// The Undo of a card delete: brings card `id` back, on `page` if it is
/// still there and through `window` (the deck may have been reopened in a
/// new page, or left). Errors go to the window's toasts.
fn undo_card_delete(
    state: &AppState,
    id: &str,
    page: &glib::WeakRef<DeckPage>,
    window: Option<&glib::WeakRef<CardsWindow>>,
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
            let message = format!("The card could not be restored: {err}");
            log_failure("a card operation", &message);
            if let Some(window) = window {
                window.add_toast(persistent_toast(&message));
            }
        }
    }
}

/// The deck page's subtitle: "3 due · 20 new", or "Nothing to study".
fn study_subtitle(input: &StudyInput) -> String {
    let due = input.reviews.len() + input.steps.len();
    let new = input.new.len();
    match (due, new) {
        (0, 0) => "Nothing to study".to_owned(),
        (due, 0) => format!("{due} due"),
        (0, new) => format!("{new} new"),
        (due, new) => format!("{due} due · {new} new"),
    }
}

/// The longest single wait before re-checking what there is to study. A
/// later change is reached in steps, so a changed clock or time zone is
/// noticed within this long.
const MAX_STUDY_WAIT_MS: u64 = 3_600_000;

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

/// Builds a [`CardRow`] per visible row and points it at its card.
fn row_factory() -> gtk::SignalListItemFactory {
    let factory = gtk::SignalListItemFactory::new();
    factory.connect_setup(|_, item| {
        if let Some(item) = item.downcast_ref::<gtk::ListItem>() {
            item.set_child(Some(&CardRow::default()));
        }
    });
    factory.connect_bind(|_, item| {
        let Some(item) = item.downcast_ref::<gtk::ListItem>() else {
            return;
        };
        if let (Some(card), Some(row)) = (
            item.item().and_downcast::<CardObject>(),
            item.child().and_downcast::<CardRow>(),
        ) {
            row.bind(&card);
            // Screen readers read the front, not "row".
            item.set_accessible_label(&card.preview());
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
    use cards_core::scheduling::session::Due;

    use super::*;

    fn due(id: &str) -> Due {
        Due {
            card_id: id.to_owned(),
            due_at: 0,
        }
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
    fn subtitle_names_what_is_left() {
        let mut input = StudyInput::default();
        assert_eq!(study_subtitle(&input), "Nothing to study");
        input.new = vec!["n".to_owned(); 20];
        assert_eq!(study_subtitle(&input), "20 new");
        input.reviews = vec![due("r1"), due("r2")];
        input.steps = vec![due("s")];
        assert_eq!(study_subtitle(&input), "3 due · 20 new");
        input.new.clear();
        assert_eq!(study_subtitle(&input), "3 due");
    }
}
