//! The All Cards page: search, filters and every card in one virtualised
//! list, with actions on several cards at once, each undone from a toast
//! (SPEC §4.6, ADR 0035, ADR 0036). Layout in `data/browser_page.blp`.
//!
//! The list model is a `gio::ListStore` of [`BrowseItem`]. Every search,
//! filter change and write reads the rows again with `Store::browse` and
//! replaces the model in one `splice`; the `GtkListView` only builds rows
//! for what is on screen, so 5,000 cards stay smooth. Selection mode, its
//! header, its action bar and the bulk actions with their Undo are
//! `ui::selection`'s, shared with a deck's page (ADR 0065).

use crate::i18n::gettext;
use crate::ui::text::with_error;
use adw::prelude::*;
use adw::subclass::prelude::*;
use gtk::{gio, glib};
use rappel_core::domain::{Card, Deck, Tag};
use rappel_core::storage::BrowseQuery;

use crate::dialogs::card_editor::{CardEditor, Saved};
use crate::dialogs::{card_info, move_card};
use crate::pages::{ReviewPage, StartedFrom};
use crate::state::AppState;
use crate::ui::selection::{Bulk, CardSelection, Parts, SelectionPage};
use crate::ui::text::one_line;
use crate::ui::toast::{info_toast, show_error, show_toast_from};
use crate::ui::{breakpoint, dropdown, init_once, nav};
use crate::widgets::{BrowseItem, CardRow, card_row};
use crate::window::RappelWindow;

pub mod text;

use crate::ui::selection::text::{EscapeAction, escape_action};
use text::Empty;

mod imp {
    use std::cell::{Cell, OnceCell, RefCell};
    use std::rc::Rc;

    use adw::prelude::*;
    use adw::subclass::prelude::*;
    use gtk::{gio, glib};
    use rappel_core::domain::{Deck, Tag};

    use crate::state::AppState;
    use crate::ui::rows;
    use crate::ui::selection::CardSelection;
    use crate::widgets::{BrowseItem, SelectionBar, SelectionHeader};

    #[derive(Debug, Default, gtk::CompositeTemplate, glib::Properties)]
    #[properties(wrapper_type = super::BrowserPage)]
    #[template(resource = "/io/github/rsmirke/Rappel/browser_page.ui")]
    pub struct BrowserPage {
        #[template_child]
        pub empty_short: TemplateChild<adw::Breakpoint>,
        #[template_child]
        pub breakpoint: TemplateChild<adw::Breakpoint>,
        #[template_child]
        pub header: TemplateChild<adw::HeaderBar>,
        #[template_child]
        pub search_entry: TemplateChild<gtk::SearchEntry>,
        #[template_child]
        pub select_button: TemplateChild<gtk::Button>,
        #[template_child]
        pub study_tag_button: TemplateChild<gtk::Button>,
        #[template_child]
        pub selection_header: TemplateChild<SelectionHeader>,
        #[template_child]
        pub deck_filter: TemplateChild<gtk::DropDown>,
        #[template_child]
        pub tag_filter: TemplateChild<gtk::DropDown>,
        #[template_child]
        pub state_filter: TemplateChild<gtk::DropDown>,
        #[template_child]
        pub archived_filter: TemplateChild<gtk::DropDown>,
        #[template_child]
        pub filter_bar: TemplateChild<adw::Clamp>,
        #[template_child]
        pub content_stack: TemplateChild<gtk::Stack>,
        #[template_child]
        pub empty_page: TemplateChild<adw::StatusPage>,
        #[template_child]
        pub empty_button: TemplateChild<gtk::Button>,
        #[template_child]
        pub count_label: TemplateChild<gtk::Label>,
        #[template_child]
        pub card_list: TemplateChild<gtk::ListView>,
        #[template_child]
        pub selection_bar: TemplateChild<SelectionBar>,
        /// A narrow window; the rows follow it.
        #[property(get, set)]
        pub compact: Cell<bool>,
        /// Cards are being chosen; the rows follow it.
        #[property(get, set)]
        pub selecting: Cell<bool>,
        /// There are two decks or more, so any card has another deck to move
        /// to: the rows' Move to Deck… is enabled (`move_card::offered`).
        #[property(get, set)]
        pub can_move: Cell<bool>,
        pub state: OnceCell<AppState>,
        pub items: OnceCell<gio::ListStore>,
        /// Selection mode: the chosen cards, the header, the action bar and
        /// the bulk actions (`ui::selection`).
        pub selection: OnceCell<CardSelection<super::BrowserPage, BrowseItem>>,
        /// The decks that are not archived: where cards can be moved.
        pub decks: RefCell<Vec<Deck>>,
        /// The decks and tags the filter drop-downs list, after "All"
        /// (archived decks only while they are shown).
        pub filter_decks: RefCell<Vec<Deck>>,
        pub tags: RefCell<Vec<Tag>>,
        /// An editor is open; a second one is not opened.
        pub editing: Rc<Cell<bool>>,
        /// The card to focus when the editor closes: the one last saved.
        pub focus_after_edit: RefCell<Option<String>>,
        /// What the list area shows (the rows, or which status page).
        pub empty: Cell<super::Empty>,
        /// The filters are being rebuilt: their changes are not searches.
        pub quiet: Cell<bool>,
        /// Something was written here: the other pages read it on leaving.
        pub dirty: Cell<bool>,
        /// A study session was started from here: the rows' statuses are
        /// read again when the page shows again.
        pub studied: Cell<bool>,
        /// The window, noted when the page is shown: a page that was popped
        /// may no longer have a root when `hidden` is emitted.
        pub window: glib::WeakRef<crate::window::RappelWindow>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for BrowserPage {
        const NAME: &'static str = "RappelBrowserPage";
        type Type = super::BrowserPage;
        type ParentType = adw::NavigationPage;

        fn class_init(klass: &mut Self::Class) {
            SelectionHeader::ensure_type();
            SelectionBar::ensure_type();
            klass.bind_template();
            // Activated by the card rows' menus with the card id.
            rows::install_id_action(klass, "browser.edit-card", super::BrowserPage::edit);
            rows::install_id_action(klass, "browser.card-info", super::BrowserPage::card_info);
            rows::install_id_action(klass, "browser.suspend-card", |page, id| {
                page.run_on(id, super::Bulk::Suspend(true));
            });
            rows::install_id_action(klass, "browser.unsuspend-card", |page, id| {
                page.run_on(id, super::Bulk::Suspend(false));
            });
            rows::install_id_action(klass, "browser.reset-card", |page, id| {
                page.run_on(id, super::Bulk::Reset);
            });
            rows::install_id_action(klass, "browser.move-card", super::BrowserPage::move_card);
            // A card's menu, with a tag's id; and the header's button, for
            // the tag the filter names (ADR 0069).
            rows::install_id_action(klass, "browser.study-tag", super::BrowserPage::study_tag);
            klass.install_action("browser.study-filter-tag", None, |page, _, _| {
                if let Some(tag) = page.filter_tag() {
                    page.study_tag(&tag.id);
                }
            });
            rows::install_id_action(klass, "browser.delete-card", |page, id| {
                page.run_on(id, super::Bulk::Delete);
            });
            // A long press on a touch screen.
            rows::install_id_action(klass, "browser.select-card", |page, id| {
                page.selection().press(id);
            });
            // The status page's button: back to the decks, or clear.
            klass.install_action("browser.empty-action", None, |page, _, _| {
                page.empty_action();
            });
        }

        fn instance_init(obj: &glib::subclass::InitializingObject<Self>) {
            obj.init_template();
        }
    }

    #[glib::derived_properties]
    impl ObjectImpl for BrowserPage {}
    impl WidgetImpl for BrowserPage {}
    impl NavigationPageImpl for BrowserPage {}
}

glib::wrapper! {
    pub struct BrowserPage(ObjectSubclass<imp::BrowserPage>)
        @extends adw::NavigationPage, gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}

impl BrowserPage {
    /// The page, listing every card; `deck_id` starts it filtered to a deck.
    ///
    /// # Errors
    /// If the decks or tags cannot be read.
    pub fn new(state: AppState, deck_id: Option<&str>) -> rappel_core::Result<Self> {
        let page: Self = glib::Object::new();
        let imp = page.imp();
        let store = gio::ListStore::new::<BrowseItem>();
        imp.card_list
            .set_model(Some(&gtk::NoSelection::new(Some(store.clone()))));
        imp.card_list.set_factory(Some(&page.row_factory()));
        // The empty state on a short window (ADR 0058); its icon changes
        // with the state, so only `compact`.
        crate::ui::breakpoint::compact_when(&imp.empty_short, &*imp.empty_page);
        dropdown::set_labels(&imp.archived_filter, &text::archived_entries());
        for filter in [
            &*imp.deck_filter,
            &*imp.tag_filter,
            &*imp.state_filter,
            &*imp.archived_filter,
        ] {
            dropdown::shrinkable(filter);
        }
        // On a phone Select All and the action bar keep only their icons.
        for widget in [
            imp.selection_header.upcast_ref::<glib::Object>(),
            imp.selection_bar.upcast_ref(),
        ] {
            page.bind_property("compact", widget, "compact")
                .sync_create()
                .build();
        }
        init_once(
            &imp.selection,
            CardSelection::new(
                &page,
                state.clone(),
                Parts {
                    own_header: imp.header.get().upcast(),
                    select_button: imp.select_button.get(),
                    narrow_select_button: None,
                    header: imp.selection_header.get(),
                    bar: imp.selection_bar.get(),
                    list: imp.card_list.get(),
                    items: store.clone(),
                    own_deck: None,
                },
            ),
        );
        // Move… follows whether there is another deck.
        page.connect_can_move_notify(|page| page.selection().update());
        init_once(&imp.state, state);
        init_once(&imp.items, store);
        page.load_filters(deck_id)?;
        page.connect_signals();
        page.load_rows()?;
        Ok(page)
    }

    /// Selection mode. Set in `new`, before anything can ask for it.
    #[expect(
        clippy::expect_used,
        reason = "an invariant: `new` sets the cell before it returns the page"
    )]
    fn selection(&self) -> &CardSelection<Self, BrowseItem> {
        self.imp()
            .selection
            .get()
            .expect("`BrowserPage::new` sets the selection before it returns the page")
    }

    /// Puts the keyboard in the search field (Ctrl+F). Not while cards are
    /// being chosen: the header shows the selection then, and Cancel or
    /// Escape comes first (the choice is not dropped by a shortcut).
    pub fn focus_search(&self) {
        if !self.selecting() {
            self.imp().search_entry.grab_focus();
        }
    }

    /// Reads the rows again, after an import.
    pub fn reload_all(&self) {
        self.reload_filters();
        self.requery();
    }

    /// Reads the decks and tags of the filters again, or says why not.
    fn reload_filters(&self) {
        if let Err(err) = self.load_filters(None) {
            // Translators: a toast: what is before ":" is its title, the rest is behind Details. {error} is why, in English.
            self.error_toast(&with_error(
                &gettext("The decks could not be read: {error}"),
                &err,
            ));
        }
    }

    /// Whether the cards of archived decks are listed.
    fn archived_shown(&self) -> bool {
        let filter = &self.imp().archived_filter;
        // The property, not `is_visible`: the page may not be shown yet.
        text::archived_shown(filter.get_visible(), filter.selected())
    }

    fn connect_signals(&self) {
        let imp = self.imp();
        breakpoint::follow(
            &imp.breakpoint,
            glib::clone!(
                #[weak(rename_to = page)]
                self,
                move |compact| {
                    page.set_compact(compact);
                }
            ),
        );
        imp.search_entry.connect_search_changed(glib::clone!(
            #[weak(rename_to = page)]
            self,
            move |_| page.search()
        ));
        // `stop-search` (Escape) has no default action (GtkSearchEntry
        // docs): clear the text, or, if there is none, leave the field.
        imp.search_entry.connect_stop_search(glib::clone!(
            #[weak(rename_to = page)]
            self,
            move |entry| {
                if entry.text().is_empty() {
                    page.leave_search();
                } else {
                    entry.set_text("");
                }
            }
        ));
        // Down in the field goes to the list.
        let down = gtk::EventControllerKey::new();
        down.connect_key_pressed(glib::clone!(
            #[weak(rename_to = page)]
            self,
            #[upgrade_or]
            glib::Propagation::Proceed,
            move |_, key, _, _| {
                if key == text::TO_LIST_KEY {
                    page.imp().card_list.grab_focus();
                    glib::Propagation::Stop
                } else {
                    glib::Propagation::Proceed
                }
            }
        ));
        imp.search_entry.add_controller(down);
        for dropdown in [&*imp.deck_filter, &*imp.tag_filter, &*imp.state_filter] {
            dropdown.connect_selected_notify(glib::clone!(
                #[weak(rename_to = page)]
                self,
                move |_| {
                    if !page.imp().quiet.get() {
                        page.search();
                    }
                }
            ));
        }
        // Showing or hiding archived decks changes what the deck filter
        // lists too.
        imp.archived_filter.connect_selected_notify(glib::clone!(
            #[weak(rename_to = page)]
            self,
            move |_| {
                if !page.imp().quiet.get() {
                    page.reload_filters();
                    page.search();
                }
            }
        ));
        imp.card_list.connect_activate(glib::clone!(
            #[weak(rename_to = page)]
            self,
            move |_, position| page.activate(position)
        ));
        self.add_type_to_search();
        self.connect_hidden(|page| {
            // The deck pages and the deck list read what was written here
            // (the page may be about to go, so it is the window's to ask).
            if page.imp().dirty.replace(false)
                && let Some(window) = page.imp().window.upgrade()
            {
                window.data_changed();
            }
        });
        self.connect_shown(|page| {
            page.imp()
                .window
                .set(page.root().and_downcast::<RappelWindow>().as_ref());
            // Back from studying: the statuses ("Due in 3 days") changed,
            // and the decks' counts, which the window reads on leaving.
            if page.imp().studied.replace(false) {
                page.imp().dirty.set(true);
                page.requery();
            }
            page.focus_search();
        });
        self.add_escape();
    }

    /// Typing in the list or on the status page searches: a key that types
    /// a character goes into the search field, which takes the focus
    /// without selecting its text, so typing, Backspace and the arrows
    /// carry on there (`text::types_into_search`). Bubble phase on the
    /// content, so the list's own keys come first, the filters and the
    /// header keep theirs, and a key typed while a popover is open (a row's
    /// menu, a drop-down's popup: the event comes from the popover's own
    /// surface) stays there. Not GTK's `gtk_search_entry_set_key_capture_widget`:
    /// it forwards keys from popovers under the capture widget too and
    /// leaves the focus behind (gtksearchentry.c,
    /// `capture_widget_key_handled`).
    fn add_type_to_search(&self) {
        let keys = gtk::EventControllerKey::new();
        keys.connect_key_pressed(glib::clone!(
            #[weak(rename_to = page)]
            self,
            #[upgrade_or]
            glib::Propagation::Proceed,
            move |keys, key, _, modifiers| {
                let imp = page.imp();
                let in_popover = keys.current_event().and_then(|event| event.surface())
                    != imp.content_stack.native().and_then(|n| n.surface());
                let search_empty = imp.search_entry.text().is_empty();
                if !text::types_into_search(
                    key,
                    modifiers,
                    in_popover,
                    page.selecting(),
                    search_empty,
                ) {
                    return glib::Propagation::Proceed;
                }
                let Some(field) = imp.search_entry.delegate().and_downcast::<gtk::Text>() else {
                    return glib::Propagation::Proceed;
                };
                field.grab_focus_without_selecting();
                field.set_position(-1);
                if keys.forward(&field) {
                    glib::Propagation::Stop
                } else {
                    glib::Propagation::Proceed
                }
            }
        ));
        self.imp().content_stack.add_controller(keys);
    }

    /// Escape while the focus is in the list. With nothing to leave or
    /// clear it is not handled, so the navigation view goes back.
    fn add_escape(&self) {
        let keys = gtk::EventControllerKey::new();
        keys.connect_key_pressed(glib::clone!(
            #[weak(rename_to = page)]
            self,
            #[upgrade_or]
            glib::Propagation::Proceed,
            move |_, key, _, _| {
                if key == text::ESCAPE_KEY {
                    page.escape()
                } else {
                    glib::Propagation::Proceed
                }
            }
        ));
        self.add_controller(keys);
    }

    /// What Escape does in the list (see [`escape_action`]).
    fn escape(&self) -> glib::Propagation {
        // Read first: the actions below borrow the selection again, and a
        // `Ref` kept in the `match` scrutinee would still be alive in them.
        let selecting = self.selection().active();
        let filtered = text::is_filtered(&self.query());
        match escape_action(selecting, filtered) {
            EscapeAction::LeaveSelection => {
                self.selection().set_active(false);
                glib::Propagation::Stop
            }
            EscapeAction::ClearSearch => {
                self.clear_filters();
                glib::Propagation::Stop
            }
            EscapeAction::Nothing => glib::Propagation::Proceed,
        }
    }

    /// Escape in an empty search field leaves it: for the list, or the
    /// button of the empty state.
    fn leave_search(&self) {
        let imp = self.imp();
        match imp.content_stack.visible_child_name().as_deref() {
            Some("list") => {
                imp.card_list.grab_focus();
            }
            Some("empty") => {
                imp.empty_button.grab_focus();
            }
            _ => {}
        }
    }

    /// A row per visible card, following the page's `compact` and
    /// `selecting` properties.
    fn row_factory(&self) -> gtk::SignalListItemFactory {
        card_row::factory(
            self,
            CardRow::for_browser,
            &["compact", "selecting", "can-move"],
            CardRow::bind_browse,
        )
    }

    // --- reading ---------------------------------------------------------

    /// Fills the deck and tag drop-downs, keeping what was chosen (or
    /// choosing `deck_id`). The tag filter lists the tags in use, and the
    /// chosen one even if no card has it any more (`text::filter_tags`,
    /// ADR 0052). The archived filter shows while some deck is archived,
    /// and the deck filter lists the archived decks, marked, while they are
    /// shown (ADR 0068).
    fn load_filters(&self, deck_id: Option<&str>) -> rappel_core::Result<()> {
        let imp = self.imp();
        let Some(state) = imp.state.get() else {
            return Ok(());
        };
        let every_deck = state.decks_with_archived()?;
        let decks: Vec<Deck> = every_deck
            .iter()
            .filter(|deck| !deck.is_archived())
            .cloned()
            .collect();
        let kept_deck =
            text::filter_choice(imp.deck_filter.selected(), imp.filter_decks.borrow().len())
                .map(|i| imp.filter_decks.borrow()[i].id.clone());
        let kept = text::filter_choice(imp.tag_filter.selected(), imp.tags.borrow().len())
            .map(|i| imp.tags.borrow()[i].clone());
        let kept_tag = kept.as_ref().map(|t| t.id.clone());
        let any_archived = every_deck.iter().any(Deck::is_archived);
        let show_archived = text::archived_shown(any_archived, imp.archived_filter.selected());
        // Hidden, archived decks lend the tag filter no tag of their own.
        let tags = text::filter_tags(state.used_tags_listed(show_archived)?, kept);
        let want_deck = deck_id.map(str::to_owned).or(kept_deck);

        imp.quiet.set(true);
        imp.archived_filter.set_visible(any_archived);
        let listed = text::filter_decks(&every_deck, show_archived);
        dropdown::set_labels(
            &imp.deck_filter,
            &std::iter::once(text::all_decks())
                .chain(listed.iter().map(text::deck_filter_label))
                .collect::<Vec<_>>(),
        );
        dropdown::set_labels(
            &imp.tag_filter,
            &labels(&text::all_tags(), tags.iter().map(|t| &t.name)),
        );
        let position =
            |found: Option<usize>| found.map_or(0, |i| u32::try_from(i + 1).unwrap_or(0));
        imp.deck_filter.set_selected(position(
            want_deck
                .as_ref()
                .and_then(|id| listed.iter().position(|d| &d.id == id)),
        ));
        imp.tag_filter.set_selected(position(
            kept_tag
                .as_ref()
                .and_then(|id| tags.iter().position(|t| &t.id == id)),
        ));
        imp.quiet.set(false);
        self.set_can_move(move_card::offered(&decks, None));
        imp.decks.replace(decks);
        imp.filter_decks.replace(listed);
        imp.tags.replace(tags);
        Ok(())
    }

    /// What the search field and the filters ask for.
    fn query(&self) -> BrowseQuery {
        let imp = self.imp();
        BrowseQuery {
            text: imp.search_entry.text().to_string(),
            deck_id: text::filter_choice(
                imp.deck_filter.selected(),
                imp.filter_decks.borrow().len(),
            )
            .map(|i| imp.filter_decks.borrow()[i].id.clone()),
            tag_id: text::filter_choice(imp.tag_filter.selected(), imp.tags.borrow().len())
                .map(|i| imp.tags.borrow()[i].id.clone()),
            state: text::state_filter(imp.state_filter.selected()),
            include_archived: self.archived_shown(),
        }
    }

    /// The user changed the words or a filter: read the rows again and tell
    /// a screen reader how many there are.
    fn search(&self) {
        self.requery();
        let imp = self.imp();
        let n = imp.items.get().map_or(0, gio::ListStore::n_items);
        // From the page: the count label is hidden on a narrow window.
        self.announce(
            &text::count_text(n as usize),
            gtk::AccessibleAnnouncementPriority::Medium,
        );
    }

    /// Reads the rows again and puts them in the list. The cards chosen
    /// stay chosen if they are still listed.
    pub fn requery(&self) {
        if let Err(err) = self.load_rows() {
            // Translators: a toast: what is before ":" is its title, the rest is behind Details. {error} is why, in English.
            self.error_toast(&with_error(
                &gettext("The cards could not be read: {error}"),
                &err,
            ));
        }
    }

    /// [`Self::requery`] that returns the error: a page that cannot read
    /// its cards must not open and say "No Cards Yet".
    fn load_rows(&self) -> rappel_core::Result<()> {
        let imp = self.imp();
        let (Some(state), Some(store)) = (imp.state.get(), imp.items.get()) else {
            return Ok(());
        };
        let query = self.query();
        let rows = state.browse(&query, rappel_core::time::now_ms())?;
        let items: Vec<BrowseItem> = rows.into_iter().map(BrowseItem::new).collect();
        store.splice(0, store.n_items(), &items);
        let filtered = text::is_filtered(&query);
        // Whether there is any card at all, whatever the filters, archived
        // decks included: read only when nothing was found while something
        // was left out (a filter, or archived decks hidden); otherwise no
        // rows means no cards.
        let collection_empty = if items.is_empty() && (filtered || !query.include_archived) {
            !state.has_live_cards()?
        } else {
            items.is_empty()
        };
        let empty = text::empty_state(items.len(), filtered, collection_empty);
        imp.empty.set(empty);
        imp.filter_bar.set_visible(text::filters_shown(empty));
        match text::empty_page(empty) {
            None => imp.content_stack.set_visible_child_name("list"),
            Some((icon, title, description)) => {
                imp.empty_page.set_icon_name(Some(icon));
                imp.empty_page.set_title(&title);
                imp.empty_page.set_description(Some(&description));
                imp.empty_button.set_label(&match empty {
                    Empty::NoCards => text::go_to_decks(),
                    Empty::OnlyArchived => text::show_archived_label(),
                    Empty::NoResults | Empty::No => {
                        let (searched, filtered) = text::searched_and_filtered(&query);
                        text::clear_label(searched, filtered)
                    }
                });
                imp.content_stack.set_visible_child_name("empty");
            }
        }
        imp.count_label.set_label(&text::count_text(items.len()));
        // Select is enabled while there is a card to choose.
        self.selection().list_changed();
        self.update_study_tag();
        Ok(())
    }

    /// The tag the tag filter names, if it names one.
    fn filter_tag(&self) -> Option<Tag> {
        let imp = self.imp();
        let tags = imp.tags.borrow();
        text::filter_choice(imp.tag_filter.selected(), tags.len()).map(|i| tags[i].clone())
    }

    /// Shows the header's Study button while the tag filter names a tag,
    /// labelled for it.
    fn update_study_tag(&self) {
        let button = &self.imp().study_tag_button;
        match self.filter_tag() {
            Some(tag) => {
                // Every deck, whatever the deck filter shows (ADR 0069).
                button.set_tooltip_text(Some(&card_row::text::study_tag_everywhere(
                    &tag.name, false,
                )));
                button.update_property(&[gtk::accessible::Property::Label(
                    &card_row::text::study_tag_everywhere(&tag.name, true),
                )]);
                button.set_visible(true);
            }
            None => button.set_visible(false),
        }
    }

    /// Pushes a session over the cards carrying tag `tag_id` in every deck
    /// that is not archived (ADR 0069): from a card's menu, or the header's
    /// button for the tag filter's. Its Back, and its summary's Back to All
    /// Cards, come back here, which then reads its rows again.
    fn study_tag(&self, tag_id: &str) {
        let Some(state) = self.imp().state.get() else {
            return;
        };
        if self.selecting() {
            return;
        }
        let on_saved = glib::clone!(
            #[weak(rename_to = page)]
            self,
            move |saved| page.saved(&saved)
        );
        match ReviewPage::push_for_tag(self, state, tag_id, StartedFrom::AllCards, on_saved) {
            // Only a session actually pushed changes what this page shows.
            Ok(pushed) => {
                if pushed {
                    self.imp().studied.set(true);
                }
            }
            Err(err) => {
                // Translators: a toast: what is before ":" is its title, the rest is behind Details. {error} is why, in English.
                self.error_toast(&with_error(
                    &gettext("The session could not start: {error}"),
                    &err,
                ));
            }
        }
    }

    fn clear_filters(&self) {
        let imp = self.imp();
        imp.quiet.set(true);
        imp.deck_filter.set_selected(0);
        imp.tag_filter.set_selected(0);
        imp.state_filter.set_selected(0);
        imp.quiet.set(false);
        // Clearing the text searches again; if it was empty nothing fires.
        imp.search_entry.set_text("");
        self.search();
        self.focus_search();
    }

    fn item(&self, position: u32) -> Option<BrowseItem> {
        self.imp().items.get()?.item(position).and_downcast()
    }

    /// Where card `id` is in the list. A linear scan, only on a user action.
    fn position(&self, id: &str) -> Option<u32> {
        let store = self.imp().items.get()?;
        (0..store.n_items()).find(|&p| self.item(p).is_some_and(|i| i.id() == id))
    }

    /// The listed cards' ids, in list order.
    #[cfg(test)]
    fn ids(&self) -> Vec<String> {
        let n = self.imp().items.get().map_or(0, gio::ListStore::n_items);
        (0..n)
            .filter_map(|p| self.item(p))
            .map(|i| i.id())
            .collect()
    }

    /// The status page's button: on "No Cards Yet" back to the deck list
    /// (where decks are made and cards added), on "No Results Found" clear
    /// the search and filters.
    fn empty_action(&self) {
        match self.imp().empty.get() {
            Empty::NoCards => nav::pop(self),
            Empty::NoResults => self.clear_filters(),
            // The drop-down's own change lists them (and their decks).
            Empty::OnlyArchived => self.imp().archived_filter.set_selected(1),
            Empty::No => {}
        }
    }

    /// A row was activated: choose it, or open its editor.
    fn activate(&self, position: u32) {
        if self.selection().activate(position) {
            return;
        }
        if let Some(item) = self.item(position) {
            self.open_editor(&item.row().card);
        }
    }

    // --- one card --------------------------------------------------------

    fn edit(&self, id: &str) {
        if let Some(item) = self.position(id).and_then(|p| self.item(p)) {
            self.open_editor(&item.row().card);
        }
    }

    fn open_editor(&self, card: &Card) {
        let imp = self.imp();
        let Some(state) = imp.state.get() else {
            return;
        };
        CardEditor::open(
            self,
            &imp.editing,
            state.clone(),
            &card.deck_id,
            Some(card),
            glib::clone!(
                #[weak(rename_to = page)]
                self,
                move |saved| page.saved(&saved)
            ),
            glib::clone!(
                #[weak(rename_to = page)]
                self,
                move || {
                    if let Some(id) = page.imp().focus_after_edit.take()
                        && let Some(position) = page.position(&id)
                    {
                        page.focus(position);
                    }
                }
            ),
        );
    }

    /// A save from the editor: the saved row is read again in place (so the
    /// list keeps its place), a card added with Ctrl+Shift+Enter reads the
    /// list again.
    fn saved(&self, saved: &Saved) {
        let imp = self.imp();
        imp.dirty.set(true);
        match saved {
            Saved::Edited(tagged) => {
                let id = &tagged.card.id;
                imp.focus_after_edit.replace(Some(id.clone()));
                let (Some(state), Some(store)) = (imp.state.get(), imp.items.get()) else {
                    return;
                };
                let (Some(position), Ok(row)) = (
                    self.position(id),
                    state.browse_row(id, rappel_core::time::now_ms()),
                ) else {
                    self.requery();
                    return;
                };
                match row {
                    Some(row) => {
                        let item = BrowseItem::new(row);
                        item.set_selected(self.selection().is_chosen(id));
                        store.splice(position, 1, &[item]);
                    }
                    None => self.requery(),
                }
                // A new tag may be in the filter's list.
                if let Err(err) = self.load_filters(None) {
                    // Translators: a toast: what is before ":" is its title, the rest is behind Details. {error} is why, in English.
                    self.error_toast(&with_error(
                        &gettext("The tags could not be read: {error}"),
                        &err,
                    ));
                }
            }
            Saved::Added(_) => {
                if let Err(err) = self.load_filters(None) {
                    // Translators: a toast: what is before ":" is its title, the rest is behind Details. {error} is why, in English.
                    self.error_toast(&with_error(
                        &gettext("The tags could not be read: {error}"),
                        &err,
                    ));
                }
                self.requery();
            }
        }
    }

    fn card_info(&self, id: &str) {
        if let Some(state) = self.imp().state.get() {
            // AdwDialog gives the focus back to the widget that had it.
            card_info::open(self, state, id, || {});
        }
    }

    fn focus(&self, position: u32) {
        self.imp()
            .card_list
            .scroll_to(position, gtk::ListScrollFlags::FOCUS, None);
    }

    // --- actions on cards ------------------------------------------------

    /// A card's menu: the bulk action on that one card, with its Undo.
    fn run_on(&self, id: &str, action: Bulk) {
        let _ = self.selection().run(&[id.to_owned()], action);
    }

    /// A card's menu: Move to Deck… for one card. Its own deck is not
    /// offered (`move_card::targets`); the move and its Undo are the bulk
    /// action's.
    fn move_card(&self, id: &str) {
        let from = self
            .position(id)
            .and_then(|p| self.item(p))
            .map(|item| item.row().card.deck_id.clone());
        let Some(targets) = move_card::card_targets(&self.imp().decks.borrow(), from.as_deref())
        else {
            return;
        };
        if targets.is_empty() {
            show_toast_from(self, info_toast(&move_card::no_other_deck_text()));
            return;
        }
        let id = id.to_owned();
        let first = self.position(&id);
        glib::spawn_future_local(glib::clone!(
            #[weak(rename_to = page)]
            self,
            async move {
                // The dialog gives the focus back to the widget that had it
                // (a row's menu button, bound to another card by then) when
                // it closes, after the answer: `run` focuses again then.
                let moved = std::rc::Rc::new(std::cell::Cell::new(false));
                let closed = glib::clone!(
                    #[weak]
                    page,
                    #[strong]
                    moved,
                    #[strong]
                    id,
                    move || {
                        if moved.get() {
                            page.selection().refocus(first, Some(&id));
                        }
                    }
                );
                if let Some(deck) = move_card::choose_deck(&page, targets, closed).await {
                    page.run_on(&id, Bulk::MoveOne { deck_id: deck.id });
                    moved.set(true);
                }
            }
        ));
    }

    /// Something was written: read the rows again, and the other pages when
    /// this one goes.
    fn changed(&self) {
        self.imp().dirty.set(true);
        // The tags in use may have changed (a tag added, taken off, or on
        // deleted cards only), and the tag filter lists those.
        if let Err(err) = self.load_filters(None) {
            // Translators: a toast: what is before ":" is its title, the rest is behind Details. {error} is why, in English.
            self.error_toast(&with_error(
                &gettext("The decks and tags could not be read: {error}"),
                &err,
            ));
        }
        self.requery();
    }

    fn error_toast(&self, message: &str) {
        show_error(self, "a card operation", message);
    }
}

/// "All …" first, then the names, each on one line.
fn labels<'a>(all: &str, names: impl Iterator<Item = &'a String>) -> Vec<String> {
    std::iter::once(all.to_owned())
        .chain(names.map(|n| one_line(n)))
        .collect()
}

impl SelectionPage for BrowserPage {
    fn selection_mode_changed(&self, active: bool) {
        // The rows follow it, and typing no longer searches.
        self.set_selecting(active);
    }

    fn cards_changed(&self) {
        self.changed();
    }

    fn reread_cards(&self) {
        self.requery();
    }

    fn list_shown(&self) -> bool {
        self.imp().content_stack.visible_child_name().as_deref() == Some("list")
    }

    fn focus_empty_state(&self) {
        self.leave_search();
    }

    fn can_move_cards(&self) -> bool {
        self.can_move()
    }
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;
    use std::rc::Rc;
    use std::time::Duration;

    use rappel_core::domain::Grade;
    use rappel_core::storage::{CardStatus, StateFilter};

    use super::*;

    use crate::ui::selection::bulk::tests::{state_with_cards, tags};
    use crate::ui::selection::perform;
    use crate::ui::selection::text::{self as bulk_text, SuspendChoice};
    use crate::ui::time::DAY_MS;

    fn page_over(state: &AppState, deck: Option<&str>) -> BrowserPage {
        BrowserPage::new(state.clone(), deck).unwrap()
    }

    fn shown(page: &BrowserPage) -> Vec<String> {
        page.ids()
    }

    /// While the tag filter names a tag, the header offers to study it,
    /// saying which in its tooltip and accessible label; it pushes a
    /// session over that tag, and the page reads its rows again once back
    /// (ADR 0069).
    #[test]
    fn a_tag_filter_offers_to_study_its_cards() {
        crate::testing::with_widgets(|| {
            let (_tmp, state, ids, _) = state_with_cards();
            let view = adw::NavigationView::new();
            let window = gtk::Window::builder().child(&view).build();
            let page = page_over(&state, None);
            view.push(&page);
            let imp = page.imp();
            let button = &imp.study_tag_button;
            assert!(!button.get_visible(), "no tag chosen");

            imp.tag_filter.set_selected(1);
            assert_eq!(
                page.filter_tag().map(|t| t.name).as_deref(),
                Some("algebra")
            );
            assert!(button.get_visible());
            assert_eq!(
                button.tooltip_text().as_deref(),
                Some("Study Cards Tagged “\u{2068}algebra\u{2069}” in Every Deck")
            );
            imp.tag_filter.set_selected(0);
            assert!(!button.get_visible());

            imp.tag_filter.set_selected(1);
            page.activate_action("browser.study-filter-tag", None)
                .unwrap();
            let review = view
                .visible_page()
                .and_downcast::<ReviewPage>()
                .expect("a session is pushed");
            assert_eq!(review.deck_id(), None);
            assert!(imp.studied.get());
            // A second activation does not stack another session.
            page.activate_action("browser.study-filter-tag", None)
                .unwrap();
            assert_eq!(view.navigation_stack().n_items(), 2);
            // A card's menu studies its tag the same way, with the tag's id.
            view.pop();
            assert!(ids.iter().all(|id| shown(&page).contains(id)));
            let tag = page.filter_tag().unwrap();
            imp.tag_filter.set_selected(0);
            page.activate_action("browser.study-tag", Some(&tag.id.to_variant()))
                .unwrap();
            assert!(view.visible_page().and_downcast::<ReviewPage>().is_some());
            window.destroy();
        });
    }

    #[test]
    fn typing_and_the_filters_narrow_the_list() {
        crate::testing::with_widgets(|| {
            let (_tmp, state, ids, maths) = state_with_cards();
            let page = page_over(&state, None);
            let imp = page.imp();
            assert_eq!(shown(&page).len(), 4);
            assert_eq!(
                imp.content_stack.visible_child_name().as_deref(),
                Some("list")
            );

            // Case, accents and prefixes are ignored.
            imp.search_entry.set_text("ALPH");
            page.search();
            assert_eq!(shown(&page).len(), 2);
            // The deck filter: entry 0 is "All Decks", then the decks by name.
            let position = imp
                .filter_decks
                .borrow()
                .iter()
                .position(|d| d.id == maths)
                .unwrap();
            imp.deck_filter
                .set_selected(u32::try_from(position + 1).unwrap());
            assert_eq!(shown(&page), [ids[0].clone()]);
            imp.search_entry.set_text("nothing like it");
            page.search();
            assert_eq!(
                imp.content_stack.visible_child_name().as_deref(),
                Some("empty")
            );
            assert_eq!(imp.empty_page.title(), "No Results Found");
            assert_eq!(
                imp.empty_button.label().as_deref(),
                Some("_Clear Search and Filters")
            );
            // Its button clears both.
            page.empty_action();
            assert_eq!(shown(&page).len(), 4);
            imp.search_entry.set_text("nothing like it");
            page.search();
            assert_eq!(imp.empty_button.label().as_deref(), Some("_Clear Search"));

            page.clear_filters();
            assert_eq!(shown(&page).len(), 4);
            assert_eq!(imp.search_entry.text(), "");
            assert_eq!(imp.deck_filter.selected(), 0);
        });
    }

    /// Presses `key` with `modifiers` on the page's Ctrl+A controller, as
    /// a key press that reached the page would. Returns whether it handled
    /// the key (a handled key goes no further).
    fn press_select_all(page: &BrowserPage, key: gtk::gdk::Key) -> bool {
        use gtk::gdk::ModifierType;
        let controller = page
            .observe_controllers()
            .iter::<glib::Object>()
            .filter_map(Result::ok)
            .filter_map(|c| c.downcast::<gtk::EventControllerKey>().ok())
            .find(|c| c.name().as_deref() == Some(crate::ui::select_all::CONTROLLER_NAME))
            .expect("the page installs its Ctrl+A controller");
        assert_eq!(
            controller.propagation_phase(),
            gtk::PropagationPhase::Capture
        );
        controller.emit_by_name::<bool>("key-pressed", &[&key, &0u32, &ModifierType::CONTROL_MASK])
    }

    #[test]
    fn ctrl_a_chooses_every_card_listed_and_enters_selection_mode() {
        crate::testing::with_widgets(|| {
            let (_tmp, state, ids, maths) = state_with_cards();
            let page = page_over(&state, Some(&maths));
            let window = gtk::Window::builder().child(&page).build();
            let imp = page.imp();
            assert!(!page.selecting());
            // With the list focused, the filter's cards (3 of 4) are chosen.
            gtk::prelude::GtkWindowExt::set_focus(&window, Some(&*imp.card_list));
            assert!(press_select_all(&page, gtk::gdk::Key::a));
            assert!(page.selecting() && page.selection().bar().is_revealed());
            assert_eq!(page.selection().header().title(), "3 Selected");
            let mut chosen = page.selection().chosen_ids();
            chosen.sort();
            let mut expected = ids.clone();
            expected.sort();
            assert_eq!(chosen, expected);
            // Again: still those 3.
            assert!(press_select_all(&page, gtk::gdk::Key::A));
            assert_eq!(page.selection().chosen_ids().len(), 3);
        });
    }

    #[test]
    fn ctrl_a_from_the_select_button_moves_the_focus_to_the_list() {
        crate::testing::with_widgets(|| {
            let (_tmp, state, _, _) = state_with_cards();
            let page = page_over(&state, None);
            let window = gtk::Window::builder().child(&page).build();
            let imp = page.imp();
            gtk::prelude::GtkWindowExt::set_focus(&window, Some(&*imp.select_button));
            assert!(press_select_all(&page, gtk::gdk::Key::a));
            assert_eq!(page.selection().chosen_ids().len(), 4);
            assert!(!imp.select_button.is_visible());
            assert!(
                gtk::prelude::GtkWindowExt::focus(&window)
                    .is_some_and(|w| w.is_ancestor(&*imp.card_list) || w == *imp.card_list),
                "the focus went to the list, not nowhere"
            );
        });
    }

    #[test]
    fn ctrl_a_in_the_search_field_selects_its_text_not_the_cards() {
        crate::testing::with_widgets(|| {
            let (_tmp, state, _, _) = state_with_cards();
            let page = page_over(&state, None);
            let window = gtk::Window::builder().child(&page).build();
            let imp = page.imp();
            let field = imp
                .search_entry
                .delegate()
                .expect("a search entry has its text");
            gtk::prelude::GtkWindowExt::set_focus(&window, Some(&field));
            assert!(
                !press_select_all(&page, gtk::gdk::Key::a),
                "the page leaves the key to the text field"
            );
            assert!(!page.selecting());
            assert_eq!(page.selection().chosen_ids().len(), 0);
        });
    }

    #[test]
    fn a_card_menu_offers_move_to_deck_with_two_decks_or_more() {
        crate::testing::with_widgets(|| {
            let (_tmp, state, _, _) = state_with_cards();
            let page = page_over(&state, None);
            assert!(page.can_move());

            // One deck: nowhere to move a card to.
            let (_tmp, one) = crate::testing::temporary_state();
            let deck = one.create_deck("Only").unwrap();
            one.add_card_with_tags(&deck.id, "q", "a", &[]).unwrap();
            let page = page_over(&one, None);
            assert!(!page.can_move());
            // A second deck, then an import or a change elsewhere.
            one.create_deck("Second").unwrap();
            page.reload_all();
            assert!(page.can_move());
        });
    }

    /// Pressing Select (Enter or Space on it) hides it, so the focus goes
    /// to the list (Ctrl+A does the same through the same code).
    #[test]
    fn pressing_select_moves_the_focus_to_the_list() {
        crate::testing::with_widgets(|| {
            let (_tmp, state, _, _) = state_with_cards();
            let page = page_over(&state, None);
            let window = gtk::Window::builder().child(&page).build();
            let imp = page.imp();
            gtk::prelude::GtkWindowExt::set_focus(&window, Some(&*imp.select_button));
            imp.select_button.emit_clicked();
            assert!(page.selecting());
            assert!(!imp.select_button.is_visible());
            assert!(
                gtk::prelude::GtkWindowExt::focus(&window)
                    .is_some_and(|w| w == *imp.card_list || w.is_ancestor(&*imp.card_list)),
                "the focus is in the list, not nowhere"
            );
        });
    }

    /// With no list shown ("No Results Found") the Select button is
    /// disabled (nothing to choose), so this is a guard: were selection
    /// mode entered with the focus in the page's own header, which hides,
    /// the focus would go to Cancel, the only thing left to take it, not
    /// nowhere. The search field stands for that header (a disabled button
    /// cannot hold the focus).
    #[test]
    fn entering_selection_mode_without_a_list_moves_the_focus_to_cancel() {
        crate::testing::with_widgets(|| {
            let (_tmp, state, _, _) = state_with_cards();
            let page = page_over(&state, None);
            let window = gtk::Window::builder().child(&page).build();
            // Presented: in a window that never was, a hidden widget stays
            // hidden when shown (observed with GTK 4.24.1), and the header
            // of selection mode must show.
            window.present();
            let imp = page.imp();
            imp.search_entry.set_text("nothing like it");
            page.search();
            assert_eq!(imp.empty_page.title(), "No Results Found");
            let field = imp
                .search_entry
                .delegate()
                .expect("a search entry has its text");
            gtk::prelude::GtkWindowExt::set_focus(&window, Some(&field));
            // What the Select button does, though its action is disabled.
            page.selection().start();
            assert!(page.selecting());
            assert_eq!(
                gtk::prelude::GtkWindowExt::focus(&window),
                Some(imp.selection_header.cancel_button().upcast())
            );
            window.destroy();
        });
    }

    /// The labels of a drop-down's entries.
    fn entries(dropdown: &gtk::DropDown) -> Vec<String> {
        let model = dropdown.model().unwrap();
        (0..model.n_items())
            .map(|i| {
                model
                    .item(i)
                    .and_downcast::<gtk::StringObject>()
                    .unwrap()
                    .string()
                    .to_string()
            })
            .collect()
    }

    /// The archived filter (ADR 0068): shown only while a deck is
    /// archived, hiding its cards by default; shown, they are listed and
    /// the deck filter offers the deck, marked. Clear Filters and Escape
    /// leave it as it is; with only archived cards, the empty state offers
    /// to show them.
    #[test]
    fn archived_decks_are_hidden_until_shown() {
        crate::testing::with_widgets(|| {
            let (_tmp, state, ids, maths) = state_with_cards();
            let page = page_over(&state, None);
            let imp = page.imp();
            assert!(!imp.archived_filter.get_visible());
            assert_eq!(shown(&page).len(), 4);

            state.archive_deck(&maths).unwrap();
            page.reload_all();
            assert!(imp.archived_filter.get_visible());
            assert_eq!(
                entries(&imp.archived_filter),
                ["Archived Hidden", "Archived Shown"]
            );
            assert_eq!(imp.archived_filter.selected(), 0);
            assert_eq!(shown(&page).len(), 1);
            assert_eq!(entries(&imp.deck_filter), ["All Decks", "Art"]);
            // Only the archived deck's cards have "algebra".
            assert_eq!(entries(&imp.tag_filter), ["All Tags"]);
            // Moving needs two decks that are not archived.
            assert!(!page.can_move());

            imp.archived_filter.set_selected(1);
            assert_eq!(shown(&page).len(), 4);
            assert_eq!(
                entries(&imp.deck_filter),
                ["All Decks", "Art", "\u{2068}Maths\u{2069} (archived)"]
            );
            assert_eq!(entries(&imp.tag_filter), ["All Tags", "algebra"]);
            imp.deck_filter.set_selected(2);
            let mut listed = shown(&page);
            listed.sort();
            let mut expected = ids.clone();
            expected.sort();
            assert_eq!(listed, expected);
            // Clear Filters keeps the archived decks shown.
            page.clear_filters();
            assert_eq!(imp.archived_filter.selected(), 1);
            assert_eq!(shown(&page).len(), 4);
            // Hidden again: the deck filter forgets the archived deck.
            imp.deck_filter.set_selected(2);
            imp.archived_filter.set_selected(0);
            assert_eq!(imp.deck_filter.selected(), 0);
            assert_eq!(shown(&page).len(), 1);
            // Not a filter for Escape: it goes back as before.
            assert!(!text::is_filtered(&page.query()));

            // Every card archived and hidden: the empty state says so and
            // its button shows them.
            let art = state.decks().unwrap()[0].id.clone();
            state.archive_deck(&art).unwrap();
            page.reload_all();
            assert_eq!(shown(&page).len(), 0);
            assert_eq!(imp.empty_page.title(), "Only Archived Cards");
            assert_eq!(
                imp.empty_button.label().as_deref(),
                Some("_Show Archived Decks")
            );
            assert!(imp.filter_bar.get_visible());
            page.empty_action();
            assert_eq!(imp.archived_filter.selected(), 1);
            assert_eq!(shown(&page).len(), 4);

            // No archived deck left: the filter goes, and every card shows.
            state.unarchive_deck(&art).unwrap();
            state.unarchive_deck(&maths).unwrap();
            page.reload_all();
            assert!(!imp.archived_filter.get_visible());
            assert_eq!(shown(&page).len(), 4);
            assert_eq!(entries(&imp.deck_filter), ["All Decks", "Art", "Maths"]);
        });
    }

    #[test]
    fn opened_from_a_deck_it_starts_filtered_to_it() {
        crate::testing::with_widgets(|| {
            let (_tmp, state, _, maths) = state_with_cards();
            let page = page_over(&state, Some(&maths));
            assert_eq!(shown(&page).len(), 3);
            assert!(text::is_filtered(&page.query()));
        });
    }

    #[test]
    fn move_is_disabled_with_one_deck() {
        crate::testing::with_widgets(|| {
            let (_tmp, state) = crate::testing::temporary_state();
            let deck = state.create_deck("Only").unwrap();
            let id = state
                .add_card_with_tags(&deck.id, "q", "a", &[])
                .unwrap()
                .card
                .id;
            let page = page_over(&state, None);
            // Rooted: a button follows its action's state only then.
            let window = gtk::Window::builder().child(&page).build();
            page.selection().set_active(true);
            page.selection().toggle(&id);
            let move_button = &page.selection().bar().buttons()[0];
            assert!(!move_button.is_sensitive(), "nowhere to move to");
            state.create_deck("Second").unwrap();
            page.reload_all();
            // Still chosen: the list was read again.
            assert_eq!(page.selection().chosen_ids(), [id]);
            assert!(move_button.is_sensitive());
            window.destroy();
        });
    }

    /// A filter that leaves no card ends selection mode and puts the
    /// focus on the status page's button.
    #[test]
    fn a_filter_that_empties_the_list_ends_selection_mode() {
        crate::testing::with_widgets(|| {
            let (_tmp, state, ids, _) = state_with_cards();
            let page = page_over(&state, None);
            let window = gtk::Window::builder().child(&page).build();
            window.present();
            let imp = page.imp();
            page.selection().set_active(true);
            page.selection().toggle(&ids[0]);
            gtk::prelude::GtkWindowExt::set_focus(&window, Some(&*imp.state_filter));
            // Suspended: no card is.
            imp.state_filter.set_selected(4);
            assert_eq!(imp.empty_page.title(), "No Results Found");
            assert!(!page.selecting() && !page.selection().active());
            assert_eq!(
                gtk::prelude::GtkWindowExt::focus(&window),
                Some(imp.empty_button.get().upcast())
            );
            window.destroy();
        });
    }

    #[test]
    fn an_empty_collection_says_so() {
        crate::testing::with_widgets(|| {
            let (_tmp, state) = crate::testing::temporary_state();
            let page = page_over(&state, None);
            let imp = page.imp();
            assert_eq!(
                imp.content_stack.visible_child_name().as_deref(),
                Some("empty")
            );
            assert_eq!(imp.empty_page.title(), "No Cards Yet");
            assert_eq!(imp.empty_button.label().as_deref(), Some("_Go to Decks"));
            assert!(!imp.filter_bar.is_visible(), "nothing to filter");
            // Nothing to choose: no selection mode, Ctrl+A does nothing.
            let _ = page.activate_action("selection.choose", None);
            assert!(!page.selecting(), "a disabled Select does nothing");
            page.selection().select_all();
            assert!(!page.selecting());
        });
    }

    #[test]
    fn opened_from_an_empty_deck_it_says_no_cards_or_no_results() {
        crate::testing::with_widgets(|| {
            let (_tmp, state) = crate::testing::temporary_state();
            let empty = state.create_deck("Empty").unwrap();
            // No card anywhere: "No Cards Yet" at once, even filtered.
            let page = page_over(&state, Some(&empty.id));
            assert_eq!(page.imp().empty_page.title(), "No Cards Yet");
            assert!(!page.imp().filter_bar.is_visible());
            // Cards elsewhere: the deck filter finds none of them.
            let other = state.create_deck("Other").unwrap();
            state
                .add_card_with_tags(&other.id, "front", "back", &[])
                .unwrap();
            let page = page_over(&state, Some(&empty.id));
            assert_eq!(page.imp().empty_page.title(), "No Results Found");
            assert!(page.imp().filter_bar.is_visible());
            let _ = page.activate_action("selection.choose", None);
            assert!(!page.selecting(), "a disabled Select does nothing");
        });
    }

    #[test]
    fn chosen_cards_are_suspended_and_selection_mode_ends() {
        crate::testing::with_widgets(|| {
            let (_tmp, state, ids, _) = state_with_cards();
            let page = page_over(&state, None);
            let imp = page.imp();
            page.selection().set_active(true);
            assert!(page.selection().bar().is_revealed());
            assert_eq!(page.selection().header().title(), "Select Cards");
            assert!(imp.selection_header.is_visible() && !imp.select_button.is_visible());
            assert!(
                !page.can_pop(),
                "Escape leaves selection mode, not the page"
            );
            page.selection().toggle(&ids[0]);
            page.selection().toggle(&ids[1]);
            assert_eq!(page.selection().header().title(), "2 Selected");
            assert_eq!(page.selection().suspend_choice(), SuspendChoice::Suspend);

            page.activate_action("selection.suspend", None).unwrap();
            assert!(!page.selection().active());
            assert!(!page.selection().bar().is_revealed());
            assert!(page.can_pop() && imp.select_button.is_visible());
            let suspended = |state: &AppState| {
                state
                    .browse(
                        &BrowseQuery {
                            state: Some(StateFilter::Suspended),
                            ..BrowseQuery::default()
                        },
                        rappel_core::time::now_ms(),
                    )
                    .unwrap()
                    .len()
            };
            assert_eq!(suspended(&state), 2);

            // Choosing only suspended cards offers Unsuspend.
            page.selection().set_active(true);
            page.selection().toggle(&ids[0]);
            assert_eq!(page.selection().suspend_choice(), SuspendChoice::Unsuspend);
            page.selection().toggle(&ids[2]);
            assert_eq!(page.selection().suspend_choice(), SuspendChoice::Suspend);
        });
    }

    #[test]
    fn reset_is_offered_only_for_cards_with_progress() {
        crate::testing::with_widgets(|| {
            let (_tmp, state, ids, _) = state_with_cards();
            let now = rappel_core::time::now_ms();
            state
                .record_review(&ids[1], Grade::Pass, Duration::ZERO, now - DAY_MS)
                .unwrap();
            // Suspended, which its status shows instead of New.
            state.set_suspended(&ids[0..1], true).unwrap();
            let page = page_over(&state, None);
            // Without a root a button stays insensitive whatever its action
            // says (seen with GTK 4.24, see the review page's `can_undo`).
            let _window = gtk::Window::builder().child(&page).build();
            let imp = page.imp();
            page.selection().set_active(true);
            page.selection().toggle(&ids[0]);
            page.selection().toggle(&ids[2]);
            assert!(
                !imp.selection_bar.reset_button().is_sensitive(),
                "only new cards chosen"
            );
            page.selection().toggle(&ids[1]);
            assert!(imp.selection_bar.reset_button().is_sensitive());
            let item = |id: &str| page.position(id).and_then(|p| page.item(p)).unwrap();
            assert!(item(&ids[0]).is_new() && item(&ids[2]).is_new());
            assert!(!item(&ids[1]).is_new());

            // Reset elsewhere: the stale row's reset writes nothing, says
            // so, and the row catches up.
            state.reset_card(&ids[1], now).unwrap();
            assert!(!item(&ids[1]).is_new());
            let log = state.review_events(&ids[1]).unwrap();
            assert_eq!(
                perform(&state, &ids[1..2], Bulk::Reset, now).unwrap().err(),
                Some(bulk_text::no_progress_text(1))
            );
            page.selection().run(&ids[1..2], Bulk::Reset);
            assert_eq!(state.review_events(&ids[1]).unwrap(), log);
            assert!(item(&ids[1]).is_new());
        });
    }

    #[test]
    fn the_header_buttons_enter_and_leave_selection_mode() {
        crate::testing::with_widgets(|| {
            let (_tmp, state, _, _) = state_with_cards();
            let page = page_over(&state, None);
            let imp = page.imp();
            let window = gtk::Window::new();
            window.set_child(Some(&page));
            window.present();
            imp.select_button.emit_clicked();
            assert!(page.selecting());
            assert!(imp.selection_header.is_visible() && !imp.header.is_visible());
            imp.selection_header.select_all_button().emit_clicked();
            assert_eq!(page.selection().chosen_ids().len(), 4);
            assert_eq!(page.selection().header().title(), "4 Selected");
            imp.selection_header.cancel_button().emit_clicked();
            assert!(!page.selecting());
            assert!(page.can_pop(), "Back and Escape go back again after Cancel");
            assert_eq!(page.selection().chosen_ids().len(), 0);
            assert!(imp.header.is_visible() && !imp.selection_header.is_visible());
            window.destroy();
        });
    }

    #[test]
    fn add_tags_puts_the_tags_of_its_entry_on_the_chosen_cards() {
        crate::testing::with_widgets(|| {
            let (_tmp, state, ids, maths) = state_with_cards();
            // A tag on another card, to be suggested.
            state
                .add_card_with_tags(&maths, "other", "back", &tags(&["Économie"]))
                .unwrap();
            let page = page_over(&state, None);
            let window = adw::Window::new();
            window.set_content(Some(&page));
            window.present();
            page.selection().set_active(true);
            page.selection().toggle(&ids[0]);
            page.selection().toggle(&ids[1]);

            let dialog = page.selection().add_tags().unwrap();
            assert_eq!(
                dialog.body(),
                "Choose the tags to put on 2 cards. Press Enter or type a comma after each one."
            );
            let entry = dialog
                .extra_child()
                .and_downcast::<crate::widgets::TagEntry>()
                .unwrap();
            // The focus goes to the entry's text; Enter there is Add once a
            // tag is there.
            assert_eq!(dialog.focus(), Some(entry.text().upcast()));
            assert!(entry.text().activates_default());
            assert!(!dialog.is_response_enabled("confirm"), "no tag yet");

            // The editor's suggestions: "eco" finds "Économie".
            entry.type_text("eco");
            let found = entry.suggest_for_test();
            assert_eq!(found.len(), 1);
            entry.accept_suggestion(0);
            assert_eq!(entry.pill_names(), ["Économie"]);
            assert!(dialog.is_response_enabled("confirm"));
            // A name too long for a tag turns Add off; a typed name counts.
            entry.type_text(&"x".repeat(300));
            assert!(!dialog.is_response_enabled("confirm"));
            entry.type_text("exam");
            assert!(dialog.is_response_enabled("confirm"));

            dialog.emit_by_name::<()>("response", &[&"confirm"]);
            let names = |id: &str| -> Vec<String> {
                state
                    .card_tags(id)
                    .unwrap()
                    .into_iter()
                    .map(|t| t.name)
                    .collect()
            };
            assert_eq!(names(&ids[0]), ["algebra", "exam", "Économie"]);
            assert_eq!(names(&ids[1]), ["algebra", "exam", "Économie"]);
            assert_eq!(names(&ids[2]), ["algebra"], "not chosen");
            assert!(
                !page.selecting(),
                "selection mode ends, as for every action"
            );

            // Cancel adds nothing.
            page.selection().set_active(true);
            page.selection().toggle(&ids[2]);
            let dialog = page.selection().add_tags().unwrap();
            let entry = dialog
                .extra_child()
                .and_downcast::<crate::widgets::TagEntry>()
                .unwrap();
            entry.type_text("later");
            dialog.emit_by_name::<()>("response", &[&"cancel"]);
            assert_eq!(names(&ids[2]), ["algebra"]);
            window.destroy();
        });
    }

    /// The on-screen keyboard shrinks the window (ADR 0058): the dialog's
    /// sheet takes the focus off the text of Add Tags (libadwaita 1.10 at
    /// 360 × 330), which used to turn the typed name into a pill. The name
    /// stays in the text, which has the focus again (ADR 0071). Fails
    /// without `keys::AfterLeave::Refocus`, checked by making it `Split`.
    #[test]
    fn shrinking_the_window_keeps_what_is_typed_in_add_tags() {
        crate::testing::with_widgets(|| {
            let (_tmp, state, ids, _) = state_with_cards();
            let page = page_over(&state, None);
            let window = adw::Window::new();
            window.set_default_size(360, 640);
            window.set_content(Some(&page));
            window.present();
            crate::testing::settle_until(|| window.height() >= 600, Duration::from_secs(5));
            page.selection().set_active(true);
            page.selection().toggle(&ids[0]);
            let dialog = page.selection().add_tags().unwrap();
            let entry = dialog
                .extra_child()
                .and_downcast::<crate::widgets::TagEntry>()
                .unwrap();
            let text = entry.text();
            assert!(
                crate::testing::settle_until(|| text.has_focus(), Duration::from_secs(5)),
                "the text has the focus when the dialog opens"
            );
            entry.set_tags(&["kept".to_owned()]);
            // Typed without a comma, as the keys would leave it.
            let mut at = text.position();
            text.insert_text("zebra", &mut at);
            text.set_position(at);
            assert_eq!(entry.typed(), "zebra");

            window.set_default_size(360, 330);
            assert!(
                crate::testing::settle_until(|| window.height() <= 330, Duration::from_secs(5)),
                "the window got shorter ({} px)",
                window.height()
            );
            // Let the leave of the focus (decided in an idle) happen.
            crate::testing::settle_until(|| false, Duration::from_millis(500));
            assert_eq!(entry.pill_names(), ["kept"], "no pill was made");
            assert_eq!(entry.typed(), "zebra", "the name is still in the text");
            assert!(text.has_focus(), "and the text has the focus");
            assert_eq!(text.position(), 5, "with the cursor after it");
            assert_eq!(text.selection_bounds(), None, "and nothing selected");
            dialog.emit_by_name::<()>("response", &[&"cancel"]);
            window.destroy();
        });
    }

    #[test]
    fn choose_all_then_leave_forgets_the_choice() {
        crate::testing::with_widgets(|| {
            let (_tmp, state, _, _) = state_with_cards();
            let page = page_over(&state, None);
            page.selection().select_all();
            assert_eq!(page.selection().chosen_ids().len(), 4);
            // The list read again keeps the choice.
            page.requery();
            assert_eq!(page.selection().chosen_ids().len(), 4);
            page.selection().set_active(false);
            assert_eq!(page.selection().chosen_ids().len(), 0);
            assert!(!page.selecting());
        });
    }

    /// The action buttons have no labelled-by relation once rooted, so
    /// their own accessible label names them (`name_by_own_label`), also on
    /// a phone, where the breakpoint empties their words. A button with an
    /// `AdwButtonContent` that is not reset has the relation, so the check
    /// can fail.
    #[test]
    fn the_action_buttons_are_named_by_their_own_labels() {
        use gtk::{AccessibleProperty, AccessibleRelation};
        crate::testing::with_widgets(|| {
            let control = gtk::Button::new();
            control.set_child(Some(&adw::ButtonContent::builder().label("Move…").build()));
            let control_window = gtk::Window::new();
            control_window.set_child(Some(&control));
            assert!(gtk::test_accessible_has_relation(
                &control,
                AccessibleRelation::LabelledBy
            ));
            control_window.destroy();

            let (_tmp, state, _, _) = state_with_cards();
            let page = page_over(&state, None);
            let imp = page.imp();
            let bar = page.selection().bar().buttons();
            let select_all = imp.selection_header.select_all_button();
            let buttons = [&select_all, &bar[0], &bar[1], &bar[2], &bar[3]];
            let check = |when: &str| {
                for button in buttons {
                    assert!(
                        !gtk::test_accessible_has_relation(button, AccessibleRelation::LabelledBy),
                        "{when}: {:?} is labelled by its words",
                        button.tooltip_text()
                    );
                    assert!(
                        gtk::test_accessible_has_property(button, AccessibleProperty::Label),
                        "{when}: {:?} has no label",
                        button.tooltip_text()
                    );
                }
            };
            let window = gtk::Window::new();
            window.set_default_size(360, 640);
            window.set_child(Some(&page));
            check("rooted");
            // Shown at 360 px, the breakpoint applies and empties the words.
            window.present();
            assert!(crate::testing::settle_until(
                || page.compact(),
                std::time::Duration::from_secs(5)
            ));
            check("narrow");
            window.destroy();
        });
    }

    #[test]
    fn a_suspended_card_reads_as_suspended() {
        crate::testing::with_widgets(|| {
            let (_tmp, state, ids, _) = state_with_cards();
            state.set_suspended(&ids[..1], true).unwrap();
            let page = page_over(&state, None);
            let item = page.position(&ids[0]).and_then(|p| page.item(p)).unwrap();
            assert_eq!(item.row().status, CardStatus::Suspended);
        });
    }

    #[test]
    fn escape_leaves_selection_mode_then_clears_the_search_without_panicking() {
        crate::testing::with_widgets(|| {
            let (_tmp, state, ids, _) = state_with_cards();
            let page = page_over(&state, None);
            let imp = page.imp();
            // Both arms borrow the selection again: a `Ref` held while they
            // run would panic inside GTK's key handler and abort.
            page.selection().set_active(true);
            assert!(!page.can_pop());
            page.selection().toggle(&ids[0]);
            assert_eq!(page.escape(), glib::Propagation::Stop);
            assert!(!page.selection().active());
            assert!(page.can_pop(), "Back and Escape go back again after Escape");

            imp.search_entry.set_text("alpha");
            assert_eq!(page.escape(), glib::Propagation::Stop);
            assert_eq!(imp.search_entry.text(), "");
            assert_eq!(page.ids().len(), 4);

            // Nothing to leave or clear: not handled, so the page goes back.
            assert_eq!(page.escape(), glib::Propagation::Proceed);
        });
    }

    #[test]
    fn an_action_on_a_list_that_is_out_of_date_changes_nothing_and_reads_again() {
        crate::testing::with_widgets(|| {
            let (_tmp, state, ids, _) = state_with_cards();
            let page = page_over(&state, None);
            assert_eq!(page.ids().len(), 4);
            // Another window deletes a card the list still shows.
            state.delete_card(&ids[2]).unwrap();
            page.selection().run(&ids, Bulk::Suspend(true));
            assert_eq!(page.ids().len(), 3, "the list was read again");
            assert!(
                !state.card(&ids[0]).unwrap().suspended,
                "nothing was written"
            );
        });
    }

    /// Slice 2's review (ADR 0073): outside selection mode, a change of one
    /// row and a refresh of the header and bar cost no walk of the list.
    /// Before, each single-row change collected every card id to drop the
    /// cards that left the choice, and each refresh read every item to
    /// find the chosen ones, even with none chosen: O(n) per change, so a
    /// thousand changes of a 10,000-card list took 8.6 s of CPU in a debug
    /// build (checked on the code without the fix); now 19 ms (bound 200 ms).
    #[test]
    fn with_nothing_chosen_a_change_of_one_row_costs_no_walk_of_the_list() {
        crate::testing::with_widgets(|| {
            const N: u32 = 10_000;
            let (_tmp, state, _deck, _ids) = crate::testing::state_with_many_cards(N as usize);
            let page = page_over(&state, None);
            let store = page.imp().items.get().unwrap().clone();
            assert_eq!(store.n_items(), N);
            let selection = page.selection();
            assert!(!selection.active());
            let (cpu, ()) = crate::testing::cpu_time_of(|| {
                for k in 0..1_000 {
                    let p = (k * 7) % N;
                    let item = page.item(p).unwrap();
                    store.splice(p, 1, &[item]);
                    selection.update();
                }
            });
            assert_eq!(store.n_items(), N);
            assert_eq!(selection.chosen_ids().len(), 0);
            crate::testing::print_timing("All Cards: 1,000 idle row changes", cpu);
            crate::testing::assert_cpu_within(cpu, Duration::from_millis(200), "1,000 row changes");
        });
    }

    /// Hardening round 4 (ADR 0073): Ctrl+A on 10,000 cards. Choosing them
    /// all is quick (CPU time, ADR 0049), the header counts them, the list
    /// still builds only the rows on screen, and Delete on all of them and
    /// its Undo work.
    #[test]
    fn ctrl_a_on_ten_thousand_cards_is_quick_and_its_bulk_action_undoes() {
        crate::testing::with_widgets(|| {
            const N: usize = 10_000;
            let (_tmp, state, _deck, ids) = crate::testing::state_with_many_cards(N);
            let page = page_over(&state, None);
            let window = gtk::Window::builder()
                .default_width(800)
                .default_height(600)
                .child(&page)
                .build();
            window.present();
            let settle = crate::testing::settle;
            settle();
            // GTK builds the rows on screen and a margin around them, never
            // one per card.
            let built = count_rows(page.upcast_ref());
            assert!((1..1_000).contains(&built), "{built} rows");
            gtk::prelude::GtkWindowExt::set_focus(&window, Some(&*page.imp().card_list));

            let (choose, pressed) =
                crate::testing::cpu_time_of(|| press_select_all(&page, gtk::gdk::Key::a));
            assert!(pressed);
            let selection = page.selection();
            assert_eq!(selection.chosen_ids().len(), N);
            assert_eq!(selection.header().title(), "10,000 Selected");
            settle();
            assert!(count_rows(page.upcast_ref()) <= built, "no row per card");

            let chosen = selection.chosen_ids();
            let (delete, _) = crate::testing::cpu_time_of(|| selection.run(&chosen, Bulk::Delete));
            assert!(!selection.active(), "selection mode ends");
            assert_eq!(page.ids().len(), 0);
            assert!(state.card(&ids[0]).is_err());
            let (undo, ()) = crate::testing::cpu_time_of(|| selection.undo_last());
            assert_eq!(page.ids().len(), N);
            assert!(state.card(&ids[N - 1]).is_ok());
            window.destroy();

            crate::testing::print_timing("All Cards: Ctrl+A on 10,000", choose);
            crate::testing::print_timing("All Cards: delete 10,000", delete);
            crate::testing::print_timing("All Cards: undo the delete", undo);
            crate::testing::assert_cpu_within(choose, Duration::from_secs(1), "Ctrl+A");
            crate::testing::assert_cpu_within(delete, Duration::from_secs(5), "delete");
            crate::testing::assert_cpu_within(undo, Duration::from_secs(5), "undo");
        });
    }

    /// Five thousand cards: the page loads, searches and filters within a
    /// bound, and the list only builds the rows that fit on screen.
    #[test]
    fn five_thousand_cards_stay_fast_and_virtual() {
        crate::testing::with_widgets(|| {
            let (_tmp, state) = crate::testing::temporary_state();
            let deck = state.create_deck("Scratch").unwrap();
            for i in 0..5_000 {
                state
                    .add_card_with_tags(
                        &deck.id,
                        &format!("Card {i}"),
                        "back",
                        &tags(&[&format!("tag {}", i % 40)]),
                    )
                    .unwrap();
            }
            // CPU time on this thread, so a loaded machine does not fail
            // the bounds (ADR 0049); the row count below is the work.
            let (load, page) =
                crate::testing::cpu_time_of(|| BrowserPage::new(state.clone(), None).unwrap());
            let imp = page.imp();
            assert_eq!(page.ids().len(), 5_000);

            let (search, ()) = crate::testing::cpu_time_of(|| {
                imp.search_entry.set_text("card 49");
                page.search();
            });
            assert_eq!(page.ids().len(), 111, "Card 49, 490-499, 4900-4999");
            imp.search_entry.set_text("");
            page.search();

            // A huge paste: 1 MiB of words and search syntax. Only the start
            // of the query is searched (ADR 0050, row 15), and the app does
            // not cut the field; GTK keeps at most 65,535 bytes in it
            // (`GTK_ENTRY_BUFFER_MAX_SIZE`, gtkentrybuffer.c). Laid out as
            // the field would be when drawn.
            let pasted = "card \"NEAR\" * OR ( x-y ".repeat(1 << 20 >> 5);
            let (paste, ()) = crate::testing::cpu_time_of(|| {
                imp.search_entry.set_text(&pasted);
                let _ = imp.search_entry.measure(gtk::Orientation::Horizontal, -1);
                page.search();
            });
            let kept = imp.search_entry.text().len();
            assert!((60_000..=usize::from(u16::MAX)).contains(&kept), "{kept}");
            assert_eq!(page.ids().len(), 0, "no card holds every word");
            imp.search_entry.set_text("");
            page.search();

            let (choose, ()) = crate::testing::cpu_time_of(|| page.selection().select_all());
            assert_eq!(page.selection().chosen_ids().len(), 5_000);
            page.selection().set_active(false);

            // On screen: only some rows exist.
            let window = gtk::Window::builder()
                .default_width(600)
                .default_height(700)
                .child(&page)
                .build();
            window.present();
            let context = glib::MainContext::default();
            for _ in 0..200 {
                if !context.iteration(false) {
                    break;
                }
            }
            let rows = count_rows(page.upcast_ref());
            window.destroy();

            crate::testing::print_timing("browser page: load 5,000", load);
            crate::testing::print_timing("browser page: search", search);
            crate::testing::print_timing("browser page: 1 MiB paste", paste);
            crate::testing::print_timing("browser page: choose all", choose);
            if std::env::var_os("RAPPEL_TIMINGS").is_some() {
                eprintln!("browser page: rows built: {rows}");
            }
            crate::testing::assert_cpu_within(load, Duration::from_secs(5), "load");
            crate::testing::assert_cpu_within(search, Duration::from_secs(2), "search");
            crate::testing::assert_cpu_within(paste, Duration::from_millis(500), "1 MiB paste");
            crate::testing::assert_cpu_within(choose, Duration::from_secs(2), "choose all");
            // The list view builds the rows in view and a few more to size itself
            // (about 200 here), never one per card.
            assert!(rows > 0 && rows < 500, "{rows} rows are built");
        });
    }

    /// Pasting, then typing at once after it, is one search, run once the
    /// field's `search-delay` (150 ms, `data/browser_page.blp`) has passed:
    /// `GtkSearchEntry` restarts that delay on each change of a non-empty
    /// text and emits `search-changed` when it runs out
    /// (`GtkSearchEntry:search-delay` docs). The page searches only on that
    /// signal and on a filter's change. The text goes in as typing and
    /// pasting put it, through the field's `GtkText`; `set_text` would
    /// first empty the field, a change `GtkSearchEntry` reports at once.
    #[test]
    fn a_paste_and_the_keys_after_it_are_one_search() {
        crate::testing::with_widgets(|| {
            let (_tmp, state) = crate::testing::temporary_state();
            let deck = state.create_deck("Deck").unwrap();
            state
                .add_card_with_tags(&deck.id, "front", "back", &[])
                .unwrap();
            let page = BrowserPage::new(state, None).unwrap();
            let entry = page.imp().search_entry.clone();
            let field = entry
                .delegate()
                .and_downcast::<gtk::Text>()
                .expect("a search entry edits through a GtkText");
            let searches = Rc::new(Cell::new(0));
            let seen = Rc::clone(&searches);
            entry.connect_search_changed(move |_| seen.set(seen.get() + 1));
            let mut position = 0;
            field.insert_text(&"word ".repeat(10_000), &mut position);
            for key in ["x", "y", "z"] {
                field.insert_text(key, &mut position);
            }
            assert_eq!(searches.get(), 0, "nothing is searched before the delay");
            assert!(crate::testing::settle_until(
                || searches.get() > 0,
                Duration::from_secs(5)
            ));
            // Nothing else is waiting to run.
            let more =
                crate::testing::settle_until(|| searches.get() > 1, Duration::from_millis(400));
            assert!(!more, "{} searches", searches.get());
            assert_eq!(page.ids().len(), 0);
        });
    }

    fn count_rows(widget: &gtk::Widget) -> usize {
        crate::testing::descendants::<CardRow>(widget).len()
    }
}
