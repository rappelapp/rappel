//! A study session of one deck, or of the cards carrying a tag in every
//! deck that is not archived ([`Studied`], ADR 0069): the front, then the
//! back, then Fail or Pass; a summary at the end. Layout in
//! `data/review_page.blp` (ADR 0014).
//!
//! The order of cards is the core's [`Session`]. Each answer is recorded
//! at once with `record_review`, so leaving early loses nothing.
//!
//! A **practice** ([`Mode::Practice`], ADR 0070) runs on the same page over
//! the core's [`Practice`] instead: lowest retrievability first, the same
//! Fail and Pass, and **nothing written**. Its answers and their Undo go
//! only to the [`Practice`] in memory (`grade_practice`, `undo_practice`,
//! which call no writing method of [`AppState`]: the tests enforce it); a
//! banner says "Practice — nothing is rescheduled" and the summary
//! "Practice: nothing was rescheduled".
//! Edit, Card Information, Suspend, Move to Deck and Delete work as in a
//! session: explicit card commands, each with its own Undo, that do not
//! reschedule, so the summary stays true.
//!
//! Keys, in the bubble phase so a focused button keeps its own Space and
//! Enter: Space or Enter shows the answer; then 1 is Fail, 2 or Space is
//! Pass; E edits the card at any time; S suspends it and Delete deletes it
//! (after a question), both at once out of the session with an Undo toast
//! (ADR 0037); R plays the card's audio from its start (ADR 0056,
//! [`replay_target`]); Ctrl+Z (and the header's Undo button)
//! takes back the last answer, repeatedly down to the session's first, also
//! from the summary (ADR 0030). The focus rests on the card between answers.
//!
//! Editing opens the deck page's `CardEditor` on the card on screen. A
//! save re-renders the card in place (the state of the reveal, the timers
//! and the session are untouched); closing without saving changes nothing.
//! The page ignores its keys while the editor is open (`editing`), whatever
//! the dialog does with key events.

use std::rc::Rc;
use std::time::{Duration, Instant};

use adw::prelude::*;
use adw::subclass::prelude::*;
use gtk::glib;
use rappel_core::content::{self, Document};
use rappel_core::domain::{CardState, Deck, Grade, Tag};
use rappel_core::scheduling::{Practice, Session, StudyScope, Summary, capped};

use crate::dialogs::card_editor::{CardEditor, Saved};
use crate::dialogs::{card_info, move_card};
use crate::i18n::{self, gettext, ngettext, pgettext};
use crate::render::{Images, audio_controls, back_shown, clear, fill, shown_part, stop_audio};
use crate::state::AppState;
use crate::ui::callback::Callback;
use crate::ui::dialog;
use crate::ui::text::{
    counted, duration_text, group_digits, isolate, one_line, whole_percent, with_error,
};
use crate::ui::tile::{Tile, set_tile};
use crate::ui::toast::{info_toast, log_failure, show_error, show_toast_from, undo_toast};
use crate::ui::undo::{self, UndoFailure};
use crate::ui::{breakpoint, init_once, keys, nav};
use crate::window::RappelWindow;

type OnSaved = dyn Fn(Saved);

mod imp {
    use std::cell::{Cell, OnceCell, RefCell};
    use std::rc::Rc;
    use std::time::Instant;

    use adw::prelude::*;
    use adw::subclass::prelude::*;
    use gtk::glib;
    use rappel_core::content::Document;

    use super::{OnSaved, Run, Studied};
    use crate::state::AppState;
    use crate::ui::callback::Callback;
    use crate::ui::toast::ErrorSlot;

    #[derive(Debug, Default, gtk::CompositeTemplate)]
    #[template(resource = "/io/github/rsmirke/Rappel/review_page.ui")]
    pub struct ReviewPage {
        #[template_child]
        pub window_title: TemplateChild<adw::WindowTitle>,
        #[template_child]
        pub banner: TemplateChild<adw::Banner>,
        #[template_child]
        pub edit_button: TemplateChild<gtk::Button>,
        #[template_child]
        pub undo_button: TemplateChild<gtk::Button>,
        #[template_child]
        pub card_menu_button: TemplateChild<gtk::MenuButton>,
        #[template_child]
        pub stack: TemplateChild<adw::ViewStack>,
        #[template_child]
        pub card_scroller: TemplateChild<gtk::ScrolledWindow>,
        #[template_child]
        pub card_area: TemplateChild<gtk::Box>,
        #[template_child]
        pub front_box: TemplateChild<gtk::Box>,
        #[template_child]
        pub back_part: TemplateChild<gtk::Box>,
        #[template_child]
        pub back_box: TemplateChild<gtk::Box>,
        #[template_child]
        pub cut_note: TemplateChild<gtk::Label>,
        #[template_child]
        pub bottom_bar: TemplateChild<adw::Clamp>,
        #[template_child]
        pub reveal_button: TemplateChild<gtk::Button>,
        #[template_child]
        pub fail_button: TemplateChild<gtk::Button>,
        #[template_child]
        pub pass_button: TemplateChild<gtk::Button>,
        #[template_child]
        pub buttons: TemplateChild<adw::ViewStack>,
        #[template_child]
        pub short_breakpoint: TemplateChild<adw::Breakpoint>,
        #[template_child]
        pub summary_page: TemplateChild<adw::StatusPage>,
        #[template_child]
        pub summary_figures: TemplateChild<gtk::Box>,
        #[template_child]
        pub cards_value: TemplateChild<gtk::Label>,
        #[template_child]
        pub cards_caption: TemplateChild<gtk::Label>,
        #[template_child]
        pub passed_value: TemplateChild<gtk::Label>,
        #[template_child]
        pub passed_caption: TemplateChild<gtk::Label>,
        #[template_child]
        pub time_tile: TemplateChild<gtk::Box>,
        #[template_child]
        pub time_value: TemplateChild<gtk::Label>,
        #[template_child]
        pub time_caption: TemplateChild<gtk::Label>,
        #[template_child]
        pub done_button: TemplateChild<gtk::Button>,
        #[template_child]
        pub practice_button: TemplateChild<gtk::Button>,
        pub state: OnceCell<AppState>,
        /// What the session studies.
        pub studied: OnceCell<Studied>,
        /// Whether there was nothing to study when the session began: the
        /// summary then says so instead of "Session Complete".
        pub started_empty: Cell<bool>,
        /// Decks archived while this tag's session was open
        /// (`super::ReviewPage::leave_deck`): their cards are never shown
        /// again, whatever brings them back (an Undo of an answer, of a
        /// Suspend, a Delete or a Move).
        pub left_decks: RefCell<std::collections::HashSet<String>>,
        /// The session, or the practice (ADR 0070).
        pub(super) session: RefCell<Option<Run>>,
        /// The card on screen.
        pub current: RefCell<Option<String>>,
        /// Its back, parsed, until it is shown.
        pub back: RefCell<Document>,
        /// Whether only the start of the front, or of the back, is shown
        /// (`render::shown_part`).
        pub front_cut: Cell<bool>,
        pub back_cut: Cell<bool>,
        /// When the current card's front was shown.
        pub shown_at: Cell<Option<Instant>>,
        pub revealed: Cell<bool>,
        /// When the back was shown.
        pub revealed_at: Cell<Option<Instant>>,
        /// The last error toast, replaced rather than stacked when saving
        /// keeps failing.
        pub errors: ErrorSlot,
        /// Whether the card editor is open.
        pub editing: Rc<Cell<bool>>,
        /// Whether the question before a delete, or the choice of a deck to
        /// move the card to, is open.
        pub asking: Cell<bool>,
        /// Another deck exists to move the card on screen to.
        pub can_move: Cell<bool>,
        /// A window under [`EDIT_IN_MENU_WIDTH`]: Edit Card is only in the
        /// Card Menu, so the title has the header's room (ADR 0071).
        pub narrow: Cell<bool>,
        /// A card is on screen (what shows the Edit and Card Menu buttons).
        pub editable: Cell<bool>,
        /// Told about every save made from this page's editor.
        pub on_saved: OnceCell<Callback<OnSaved>>,
        /// Fades the back in at the reveal (`super::reveal_step`).
        pub reveal_animation: OnceCell<adw::TimedAnimation>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for ReviewPage {
        const NAME: &'static str = "RappelReviewPage";
        type Type = super::ReviewPage;
        type ParentType = adw::NavigationPage;

        fn class_init(klass: &mut Self::Class) {
            klass.bind_template();
            klass.install_action("review.reveal", None, |page, _, _| page.reveal());
            klass.install_action("review.fail", None, |page, _, _| {
                page.grade(rappel_core::domain::Grade::Fail);
            });
            klass.install_action("review.pass", None, |page, _, _| {
                page.grade(rappel_core::domain::Grade::Pass);
            });
            klass.install_action("review.done", None, |page, _, _| page.done());
            klass.install_action("review.edit", None, |page, _, _| page.edit());
            klass.install_action("review.card-info", None, |page, _, _| page.card_info());
            klass.install_action("review.suspend", None, |page, _, _| {
                page.remove_current(super::Removal::Suspend);
            });
            klass.install_action("review.move", None, |page, _, _| page.move_current());
            klass.install_action("review.delete", None, |page, _, _| page.confirm_delete());
            klass.install_action("review.undo", None, |page, _, _| page.undo());
            klass.install_action("review.practice", None, |page, _, _| {
                page.practise_instead();
            });
        }

        fn instance_init(obj: &glib::subclass::InitializingObject<Self>) {
            obj.init_template();
        }
    }

    impl ObjectImpl for ReviewPage {
        fn constructed(&self) {
            self.parent_constructed();
            let page = self.obj();
            page.setup_keys();
            page.setup_key_tooltips();
            page.setup_reveal_animation();
            page.setup_short_summary();
            page.follow_width();
            // A grab before the push does nothing, and the navigation view
            // then focuses the first widget: the Back button, which would
            // take Space and Enter. So park the focus once on screen.
            page.connect_shown(super::ReviewPage::park_focus);
        }
    }
    impl WidgetImpl for ReviewPage {}
    impl NavigationPageImpl for ReviewPage {}
}

glib::wrapper! {
    pub struct ReviewPage(ObjectSubclass<imp::ReviewPage>)
        @extends adw::NavigationPage, gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}

/// How long after the reveal the grade keys are ignored, against key
/// auto-repeat. Clicks are not delayed.
const KEY_SETTLE: Duration = Duration::from_millis(300);

/// How a card leaves the session.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Removal {
    Suspend,
    Delete,
}

impl Removal {
    /// The toast that offers Undo.
    fn text(self) -> String {
        match self {
            Self::Suspend => gettext("Card suspended"),
            Self::Delete => gettext("Card deleted"),
        }
    }
}

/// The Undo of a suspend or a delete: the card is studied again. On `page`
/// if it is still shown (the card becomes the next one), and through
/// `window` the other pages read it (the deck may have been left).
fn undo_removal(
    state: &AppState,
    id: &str,
    removal: Removal,
    page: &glib::WeakRef<ReviewPage>,
    window: Option<&glib::WeakRef<RappelWindow>>,
) {
    let window = window.and_then(glib::WeakRef::upgrade);
    let result = match removal {
        Removal::Suspend => state.restore_suspended(&[id.to_owned()], false).map(|_| ()),
        Removal::Delete => state.undelete_card(id).map(|_| ()),
    };
    finish_undo(state, id, result, page, window);
}

/// The Undo of a move to another deck: the card is studied again if it went
/// back to its deck. `result` is how many cards went back, 0 if it could
/// not (moved again, deleted, or its deck deleted since).
fn undo_move(
    state: &AppState,
    id: &str,
    result: rappel_core::Result<usize>,
    page: &glib::WeakRef<ReviewPage>,
    window: Option<&glib::WeakRef<RappelWindow>>,
) {
    let window = window.and_then(glib::WeakRef::upgrade);
    match result {
        Ok(0) => {
            let text = move_card::not_moved_back_text();
            match window {
                Some(window) => show_toast_from(&window, info_toast(&text)),
                None => log_failure("a review operation", &text),
            }
        }
        Ok(_) => finish_undo(state, id, Ok(()), page, window),
        Err(err) => finish_undo(state, id, Err(err), page, window),
    }
}

/// After the Undo of a removal wrote (`result`): the card is put back into
/// the session if the page is still shown and the card is live and not
/// suspended, and the other pages read the change; or the error is shown.
fn finish_undo(
    state: &AppState,
    id: &str,
    result: rappel_core::Result<()>,
    page: &glib::WeakRef<ReviewPage>,
    window: Option<RappelWindow>,
) {
    match result {
        Ok(()) => {
            // Only a card that is live and not suspended is studied again
            // (it may have been changed again since).
            let studyable = state.card(id).is_ok_and(|card| !card.suspended);
            if studyable && let Some(page) = page.upgrade().filter(|p| p.root().is_some()) {
                page.put_back(id);
            }
            if let Some(window) = window {
                window.data_changed();
            }
        }
        Err(err) => {
            // Translators: a toast: what is before ":" is its title, the rest is behind Details. {error} is why, in English.
            let message = with_error(&gettext("The card could not be restored: {error}"), &err);
            match window {
                Some(window) => show_error(&window, "a review operation", &message),
                None => log_failure("a review operation", &message),
            }
        }
    }
}

/// Which buttons and actions the bottom bar offers.
#[derive(Debug, Clone, Copy)]
enum Controls {
    /// Show Answer.
    Reveal,
    /// Fail and Pass.
    Grade,
    /// Nothing: the summary is shown.
    Hidden,
}

/// What a key does on this page.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Key {
    /// Space: show the answer, or Pass once it is shown.
    Space,
    /// Enter: show the answer.
    Enter,
    Fail,
    Pass,
    /// E: edit the card.
    Edit,
    /// I: show the card's information.
    Info,
    /// S: suspend the card.
    Suspend,
    /// Delete: delete the card, after a question.
    Delete,
    /// R: play the card's audio from its start.
    Replay,
    /// Ctrl+Z: take back the last answer.
    Undo,
}

/// The page's keys (`setup_keys`). The Keyboard Shortcuts dialog and the
/// Card Menu's hints list them; tests check both.
const KEYS: [(&str, Key); 18] = [
    ("space", Key::Space),
    ("Return", Key::Enter),
    ("KP_Enter", Key::Enter),
    ("1", Key::Fail),
    ("KP_1", Key::Fail),
    ("2", Key::Pass),
    ("KP_2", Key::Pass),
    ("e", Key::Edit),
    ("<Shift>e", Key::Edit),
    ("i", Key::Info),
    ("<Shift>i", Key::Info),
    ("s", Key::Suspend),
    ("<Shift>s", Key::Suspend),
    ("Delete", Key::Delete),
    ("KP_Delete", Key::Delete),
    ("r", Key::Replay),
    ("<Shift>r", Key::Replay),
    ("<Control>z", Key::Undo),
];

/// The triggers of [`KEYS`], for the Keyboard Shortcuts dialog's test.
#[cfg(test)]
pub(crate) fn key_triggers() -> impl Iterator<Item = &'static str> {
    KEYS.iter().map(|(trigger, _)| *trigger)
}

/// How GTK labels the first of [`KEYS`] bound to `key` ("Space", "2"), the
/// way it shows an accelerator in a menu, in the user's language.
fn key_label(key: Key) -> Option<String> {
    let (trigger, _) = KEYS.iter().find(|(_, bound)| *bound == key)?;
    keys::label(trigger)
}

/// What a key makes the page do.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Act {
    Reveal,
    Grade(Grade),
    Edit,
    Info,
    Suspend,
    Delete,
    Replay,
    Undo,
}

/// What `key` does when the answer is (not) `revealed`. `settled` says
/// whether [`KEY_SETTLE`] has passed since the reveal. Holding Space a
/// moment too long repeats it: a reveal must not become a Pass by itself,
/// even though a grade can be undone. E and Ctrl+Z are never delayed: they
/// grade nothing; neither do I and R. S and Delete take the card out of the
/// session, so they are not delayed by `key_action` but guarded elsewhere:
/// Delete asks first (the question stops the repeat), and S needs the card to
/// have been on screen a moment ([`suspend_allowed`]), or holding it would
/// suspend one card after another.
fn key_action(key: Key, revealed: bool, settled: bool) -> Option<Act> {
    match (key, revealed) {
        (Key::Edit, _) => Some(Act::Edit),
        (Key::Info, _) => Some(Act::Info),
        (Key::Suspend, _) => Some(Act::Suspend),
        (Key::Delete, _) => Some(Act::Delete),
        (Key::Undo, _) => Some(Act::Undo),
        (Key::Replay, _) => Some(Act::Replay),
        (Key::Space | Key::Enter, false) => Some(Act::Reveal),
        (Key::Space | Key::Pass | Key::Fail, true) if !settled => None,
        (Key::Space | Key::Pass, true) => Some(Act::Grade(Grade::Pass)),
        (Key::Fail, true) => Some(Act::Grade(Grade::Fail)),
        // 1 and 2 before the answer, Enter after it: nothing.
        (Key::Fail | Key::Pass, false) | (Key::Enter, true) => None,
    }
}

/// Which audio control R plays: the first that can play on the back once
/// the answer is shown and the back has audio controls, else the first
/// that can play on the front. `front` and `back` say, for each control of
/// that side in document order, whether it can play (its file is not
/// known to be missing). The back once shown, because it is what was just
/// revealed (a word to hear after reading it); a side whose controls all
/// cannot play gives nothing, not the other side's, so R never plays
/// something other than what is under the eyes.
fn replay_target(revealed: bool, front: &[bool], back: &[bool]) -> Option<(Side, usize)> {
    let (side, controls) = if revealed && !back.is_empty() {
        (Side::Back, back)
    } else {
        (Side::Front, front)
    };
    controls
        .iter()
        .position(|&available| available)
        .map(|i| (side, i))
}

/// A side of the card on screen.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Side {
    Front,
    Back,
}

/// Whether S may suspend a card that has been on screen for `shown_for`.
/// After the first S the next card is on screen, and key repeat would
/// suspend it too: a card must be shown for [`KEY_SETTLE`] first. Clicking
/// the menu item is not delayed.
fn suspend_allowed(shown_for: Option<Duration>) -> bool {
    shown_for.is_some_and(|t| t >= KEY_SETTLE)
}

impl ReviewPage {
    /// The id of the deck being studied; `None` for a tag's session, which
    /// is no deck's (the window never remembers it as the last deck, and
    /// archiving a deck does not leave it).
    pub fn deck_id(&self) -> Option<String> {
        match self.imp().studied.get()? {
            Studied::Deck { deck, .. } => Some(deck.id.clone()),
            Studied::Tag { .. } => None,
        }
    }

    /// A session over what `deck` has to study now, showing its first card.
    /// `on_saved` is called after each save made from the session's editor.
    ///
    /// # Errors
    /// If what to study cannot be read.
    pub fn new(
        state: AppState,
        deck: Deck,
        on_saved: impl Fn(Saved) + 'static,
    ) -> rappel_core::Result<Self> {
        let studied = Studied::Deck {
            deck,
            from: StartedFrom::DeckPage,
        };
        Self::start(state, studied, Mode::Study, on_saved)
    }

    /// A practice of `studied` (ADR 0070): its cards, lowest retrievability
    /// first, with nothing written; otherwise as [`Self::new`].
    ///
    /// # Errors
    /// If what to practise cannot be read.
    pub fn practice(
        state: AppState,
        studied: Studied,
        on_saved: impl Fn(Saved) + 'static,
    ) -> rappel_core::Result<Self> {
        Self::start(state, studied, Mode::Practice, on_saved)
    }

    /// Whether this page is a practice.
    pub fn is_practice(&self) -> bool {
        matches!(&*self.imp().session.borrow(), Some(Run::Practice(_)))
    }

    /// A session over the cards carrying `tag` in every deck that is not
    /// archived, each deck within its own daily limits (ADR 0069), started
    /// from `from`; otherwise as [`Self::new`].
    ///
    /// # Errors
    /// If what to study cannot be read.
    pub fn for_tag(
        state: AppState,
        tag: Tag,
        from: StartedFrom,
        on_saved: impl Fn(Saved) + 'static,
    ) -> rappel_core::Result<Self> {
        Self::start(state, Studied::Tag { tag, from }, Mode::Study, on_saved)
    }

    /// Pushes a session over tag `tag_id` above the page that holds
    /// `from_page` (a deck page or All Cards, as `from` says), unless a
    /// session is already on top. The tag is read first: `TagNotFound` if
    /// it is not one (tags are never deleted, ADR 0029).
    ///
    /// Returns whether a session was pushed.
    ///
    /// # Errors
    /// If the tag or what to study cannot be read; nothing is pushed then.
    pub fn push_for_tag(
        from_page: &impl IsA<gtk::Widget>,
        state: &AppState,
        tag_id: &str,
        from: StartedFrom,
        on_saved: impl Fn(Saved) + 'static,
    ) -> rappel_core::Result<bool> {
        let Some(view) = nav::navigation_view(from_page) else {
            return Ok(false);
        };
        nav::push_unless_on_top(&view, || {
            let tag = state
                .tags()?
                .into_iter()
                .find(|t| t.id == tag_id)
                .ok_or_else(|| rappel_core::Error::TagNotFound(tag_id.to_owned()))?;
            Self::for_tag(state.clone(), tag, from, on_saved)
        })
    }

    /// Deck `deck_id` was archived while this page is open: a tag's session
    /// drops that deck's cards, as an archived deck is not studied (ADR
    /// 0068, 0069), and the card on screen, if it is one of them, leaves
    /// the session as a suspended card does (nothing is counted). A deck's
    /// own session is left by the window instead.
    pub fn leave_deck(&self, deck_id: &str) {
        let imp = self.imp();
        let (Some(state), Some(Studied::Tag { .. })) = (imp.state.get(), imp.studied.get()) else {
            return;
        };
        imp.left_decks.borrow_mut().insert(deck_id.to_owned());
        let cards = match state.list_cards(deck_id) {
            Ok(cards) => cards,
            Err(err) => {
                tracing::warn!("cannot read the cards of an archived deck: {err}");
                return;
            }
        };
        let current = imp.current.borrow().clone();
        let mut leaves_screen = false;
        if let Some(session) = imp.session.borrow_mut().as_mut() {
            for card in &cards {
                if current.as_deref() == Some(card.id.as_str()) {
                    leaves_screen = true;
                } else {
                    session.skip(&card.id);
                }
            }
        }
        if leaves_screen && let Some(id) = current {
            self.leave_session(&id);
        }
    }

    fn start(
        state: AppState,
        studied: Studied,
        mode: Mode,
        on_saved: impl Fn(Saved) + 'static,
    ) -> rappel_core::Result<Self> {
        let now = rappel_core::time::now_ms();
        let (run, empty) = match mode {
            Mode::Study => {
                let input = state.study_input(&studied.scope(), now)?;
                let empty = input.is_empty();
                (Run::Study(Session::new(input)), empty)
            }
            Mode::Practice => {
                let queue = state.practice_queue(&studied.scope(), now)?;
                let empty = queue.is_empty();
                (Run::Practice(Practice::new(queue)), empty)
            }
        };
        let page: Self = glib::Object::new();
        let imp = page.imp();
        // Deck and tag names are untrusted; AdwWindowTitle shows plain
        // text.
        imp.window_title.set_title(&studied.title());
        imp.done_button.set_label(&studied.done_label());
        if mode == Mode::Practice {
            // Read by screen readers as the page's name, and by the Back
            // button of a page above it.
            page.set_title(&pgettext("page title", "Practice"));
        }
        imp.started_empty.set(empty);
        imp.session.replace(Some(run));
        init_once(&imp.state, state);
        init_once(&imp.studied, studied);
        init_once(&imp.on_saved, Callback::new(Rc::new(on_saved)));
        page.show_next();
        Ok(page)
    }

    fn setup_keys(&self) {
        let shortcuts = gtk::ShortcutController::new();
        // Bubble: a focused button handles its own Space and Enter first.
        shortcuts.set_propagation_phase(gtk::PropagationPhase::Bubble);
        for (trigger, key) in KEYS {
            let action = gtk::CallbackAction::new(move |widget, _| {
                widget
                    .downcast_ref::<Self>()
                    .map_or(glib::Propagation::Proceed, |page| page.key(key))
            });
            shortcuts.add_shortcut(gtk::Shortcut::new(
                gtk::ShortcutTrigger::parse_string(trigger),
                Some(action),
            ));
        }
        self.add_controller(shortcuts);
    }

    /// Gives Show Answer, Fail and Pass their key in the tooltip (#105),
    /// as the page binds it: the key's label is the one GTK shows in menus
    /// (`gtk_accelerator_get_label`), in the user's language.
    fn setup_key_tooltips(&self) {
        let imp = self.imp();
        for (button, name, key) in [
            (&*imp.reveal_button, gettext("Show Answer"), Key::Space),
            (&*imp.fail_button, gettext("Fail"), Key::Fail),
            (&*imp.pass_button, gettext("Pass"), Key::Pass),
            (&*imp.undo_button, gettext("Undo Last Answer"), Key::Undo),
            (&*imp.edit_button, gettext("Edit Card"), Key::Edit),
        ] {
            if let Some(label) = key_label(key) {
                button.set_tooltip_text(Some(&keys::tooltip(&name, &label)));
            }
        }
    }

    /// Puts the keyboard focus on the card, or on Back to Deck at the end.
    fn park_focus(&self) {
        let imp = self.imp();
        if imp.current.borrow().is_some() {
            imp.card_area.grab_focus();
        } else {
            imp.done_button.grab_focus();
        }
    }

    fn key(&self, key: Key) -> glib::Propagation {
        let imp = self.imp();
        if imp.editing.get() || imp.asking.get() {
            return glib::Propagation::Proceed;
        }
        // Also on the summary, where no card is on screen.
        if key == Key::Undo {
            self.undo();
            return glib::Propagation::Stop;
        }
        if imp.current.borrow().is_none() {
            return glib::Propagation::Proceed;
        }
        let settled = imp
            .revealed_at
            .get()
            .is_some_and(|t| t.elapsed() >= KEY_SETTLE);
        match key_action(key, imp.revealed.get(), settled) {
            Some(Act::Reveal) => self.reveal(),
            Some(Act::Grade(grade)) => self.grade(grade),
            Some(Act::Edit) => self.edit(),
            Some(Act::Info) => self.card_info(),
            Some(Act::Suspend) => {
                if suspend_allowed(imp.shown_at.get().map(|t| t.elapsed())) {
                    self.remove_current(Removal::Suspend);
                }
            }
            Some(Act::Delete) => self.confirm_delete(),
            Some(Act::Replay) => self.replay_audio(),
            Some(Act::Undo) => self.undo(),
            None => {}
        }
        glib::Propagation::Stop
    }

    /// Shows the next card, or the summary when the session is over.
    fn show_next(&self) {
        let imp = self.imp();
        let Some(state) = imp.state.get() else {
            return;
        };
        // A loop and not a recursion: a database that stays unreadable skips
        // every card of the queue, which may be thousands.
        let (id, card) = loop {
            let now = rappel_core::time::now_ms();
            let next = imp
                .session
                .borrow()
                .as_ref()
                .and_then(|s| s.next(now).map(str::to_owned));
            let Some(id) = next else {
                self.show_summary();
                return;
            };
            let card = match state.card(&id) {
                Ok(card) => card,
                Err(err) => {
                    // Skip it and go on; it is not counted as answered.
                    // Translators: a toast: what is before ":" is its title, the rest is behind Details. {error} is why, in English.
                    self.error_toast(&with_error(
                        &gettext("A card could not be read and was skipped: {error}"),
                        &err,
                    ));
                    self.skip_in_session(&id);
                    continue;
                }
            };
            // Suspended since the session began (another window, or an Undo
            // of an answer that brought it back), or of a deck archived
            // since (ADR 0069): not studied, not counted.
            if card.suspended || imp.left_decks.borrow().contains(&card.deck_id) {
                self.skip_in_session(&id);
                continue;
            }
            break (id, card);
        };
        imp.revealed.set(false);
        // A reveal still fading in ends now, out of sight.
        if let Some(animation) = imp.reveal_animation.get() {
            animation.skip();
        }
        self.show_front(&card.front_md);
        clear(&imp.back_box);
        imp.back_part.set_visible(false);
        imp.card_area
            .reset_property(gtk::AccessibleProperty::Description);
        self.keep_back(&card.back_md);
        imp.revealed_at.set(None);
        // Move to Deck… needs a deck other than the card's own.
        imp.can_move.set(
            state
                .decks()
                .is_ok_and(|decks| move_card::offered(&decks, Some(&card.deck_id))),
        );
        imp.current.replace(Some(id));
        imp.shown_at.set(Some(Instant::now()));
        let left = imp.session.borrow().as_ref().map_or(0, Run::remaining);
        imp.window_title.set_subtitle(&left_text(left));
        self.set_controls(Controls::Reveal);
        // A practice says so above every card (ADR 0070).
        imp.banner.set_revealed(self.is_practice());
        imp.stack.set_visible_child_name("card");
        imp.card_area.grab_focus();
    }

    /// Makes the animation that brings the answer in: a short fade with a
    /// small slide up ([`reveal_step`]). libadwaita skips it to its end when
    /// animations are off (the system's reduced-motion setting turns off
    /// `gtk-enable-animations`) or the widget is not mapped (`AdwAnimation`
    /// docs), so the back is then at once fully shown.
    fn setup_reveal_animation(&self) {
        let imp = self.imp();
        let back = imp.back_part.downgrade();
        let target = adw::CallbackAnimationTarget::new(move |progress| {
            if let Some(back) = back.upgrade() {
                let (opacity, margin) = reveal_step(progress);
                back.set_opacity(opacity);
                back.set_margin_top(margin);
            }
        });
        // On the card area, which is mapped whenever a card is on screen
        // (the back itself is mapped only once it is made visible).
        let animation = adw::TimedAnimation::new(&*imp.card_area, 0.0, 1.0, REVEAL_MS, target);
        animation.set_easing(adw::Easing::EaseOutCubic);
        init_once(&imp.reveal_animation, animation);
    }

    /// Keeps Edit Card out of the header on a narrow window.
    fn follow_width(&self) {
        breakpoint::follow_window_width(
            self,
            EDIT_IN_MENU_WIDTH,
            glib::clone!(
                #[weak(rename_to = page)]
                self,
                move |narrow| {
                    page.imp().narrow.set(narrow);
                    page.show_header_buttons();
                }
            ),
        );
    }

    /// The header's Edit Card and Card Menu buttons: with a card on
    /// screen; Edit Card only while the window is not narrow (see
    /// [`edit_button_shown`]).
    fn show_header_buttons(&self) {
        let imp = self.imp();
        let (editable, narrow) = (imp.editable.get(), imp.narrow.get());
        imp.edit_button
            .set_visible(edit_button_shown(editable, narrow));
        imp.card_menu_button.set_visible(editable);
    }

    /// Makes the summary `compact` (libadwaita's style class for a status
    /// page in a small space: less spacing) and drops its icon while the
    /// short-window breakpoint applies, so its figures and Back to Deck fit
    /// at 360 × 294.
    fn setup_short_summary(&self) {
        let imp = self.imp();
        breakpoint::compact_status_page(&imp.short_breakpoint, &imp.summary_page, SUMMARY_ICON);
    }

    /// Fades the back in, from its first step: `play` restarts the
    /// animation, but the first frame is drawn before its first tick, so
    /// the back starts out hidden here.
    fn animate_reveal(&self) {
        let imp = self.imp();
        let (opacity, margin) = reveal_step(0.0);
        imp.back_part.set_opacity(opacity);
        imp.back_part.set_margin_top(margin);
        if let Some(animation) = imp.reveal_animation.get() {
            animation.play();
        }
    }

    fn skip_in_session(&self, id: &str) {
        if let Some(session) = self.imp().session.borrow_mut().as_mut() {
            session.skip(id);
        }
    }

    fn reveal(&self) {
        let imp = self.imp();
        if imp.current.borrow().is_none() || imp.revealed.replace(true) {
            return;
        }
        imp.revealed_at.set(Some(Instant::now()));
        let back = imp.back.take();
        // Only the reveal animates; a redraw after an edit does not.
        if self.show_back(&back) {
            self.animate_reveal();
            self.scroll_to_answer_when_laid_out();
        }
        self.set_controls(Controls::Grade);
        imp.card_area.grab_focus();
    }

    /// Once the answer is laid out (the next frame: a tick callback runs
    /// before that frame's layout, so the second one sees it), scrolls the
    /// card so the answer is in view if it starts too low to be seen
    /// (`answer_scroll`). A phone with a tall front would otherwise show
    /// the grade buttons with no answer on screen.
    fn scroll_to_answer_when_laid_out(&self) {
        let frames = std::cell::Cell::new(0);
        self.add_tick_callback(move |page, _| {
            frames.set(frames.get() + 1);
            if frames.get() < 2 {
                return glib::ControlFlow::Continue;
            }
            let imp = page.imp();
            if imp.revealed.get()
                && let Some(bounds) = imp.back_part.compute_bounds(&*imp.card_scroller)
            {
                let adjustment = imp.card_scroller.vadjustment();
                let delta = answer_scroll(f64::from(bounds.y()), adjustment.page_size());
                if delta != 0.0 {
                    adjustment.set_value(adjustment.value() + delta);
                }
            }
            glib::ControlFlow::Break
        });
    }

    /// Shows the front `front_md`, or its start if it is too long to lay
    /// out (`render::shown_part`). Screen readers read it when the card
    /// area has the focus.
    fn show_front(&self, front_md: &str) {
        let imp = self.imp();
        let (part, cut) = shown_part(front_md);
        let front = content::parse(part);
        fill(&imp.front_box, &front, &self.images());
        imp.card_area
            .update_property(&[gtk::accessible::Property::Label(&with_cut_note(
                content::plain_text(&front),
                cut,
            ))]);
        imp.front_cut.set(cut);
        self.update_cut_note();
    }

    /// Where the sides' images come from, and their height cap: a share
    /// of the page's height, so a tall image leaves the text around it in
    /// view (`render::Images`). The grade buttons are in the bottom bar,
    /// outside the scrolled card, so no image can push them off screen.
    fn images(&self) -> Images {
        let imp = self.imp();
        let folder = imp.state.get().and_then(AppState::media_folder);
        // The card's own view, between the header and the buttons: on a
        // phone held sideways the page is 360 px tall but the card has
        // about 240 (ADR 0058). Before the first layout, the page's.
        let view = match imp.card_scroller.height() {
            0 => self.height(),
            height => height,
        };
        Images::new(folder, view)
    }

    /// Parses the back `back_md` (or its start, as for the front) for
    /// [`Self::reveal`].
    fn keep_back(&self, back_md: &str) {
        let imp = self.imp();
        let (part, cut) = shown_part(back_md);
        imp.back.replace(content::parse(part));
        imp.back_cut.set(cut);
    }

    /// Shows the note under the card while a side on screen is cut.
    fn update_cut_note(&self) {
        let imp = self.imp();
        imp.cut_note.set_visible(cut_note_shown(
            imp.front_cut.get(),
            imp.back_cut.get(),
            imp.revealed.get(),
        ));
    }

    /// Shows `back` under the front, or nothing when it is empty: then
    /// only the grade buttons appear. Says whether it is shown.
    fn show_back(&self, back: &Document) -> bool {
        let imp = self.imp();
        let shown = back_shown(back);
        if shown {
            fill(&imp.back_box, back, &self.images());
            imp.card_area
                .update_property(&[gtk::accessible::Property::Description(&with_cut_note(
                    content::plain_text(back),
                    imp.back_cut.get(),
                ))]);
        } else {
            clear(&imp.back_box);
            imp.card_area
                .reset_property(gtk::AccessibleProperty::Description);
            // A cut back whose start has nothing to show (only blank
            // lines) is not on screen, so the note is not about it.
            imp.back_cut.set(false);
        }
        imp.back_part.set_visible(shown);
        self.update_cut_note();
        shown
    }

    /// Opens the card editor on the card on screen.
    fn edit(&self) {
        let imp = self.imp();
        let (Some(state), Some(id)) = (imp.state.get(), imp.current.borrow().clone()) else {
            return;
        };
        if imp.editing.get() {
            return;
        }
        // The editor covers the card: what it played stops.
        stop_audio();
        // Read again, so the editor starts from what is stored.
        let card = match state.card(&id) {
            Ok(card) => card,
            Err(err) => {
                // Translators: a toast: what is before ":" is its title, the rest is behind Details. {error} is why, in English.
                self.error_toast(&with_error(
                    &gettext("The card could not be read: {error}"),
                    &err,
                ));
                return;
            }
        };
        CardEditor::open(
            self,
            &imp.editing,
            state.clone(),
            // The card's own deck: in a tag's session it is not one deck
            // (Ctrl+Shift+Enter adds the next card there too).
            &card.deck_id,
            Some(&card),
            glib::clone!(
                #[weak(rename_to = page)]
                self,
                move |saved| page.saved(saved)
            ),
            glib::clone!(
                #[weak(rename_to = page)]
                self,
                // Set here, not left to whatever focus the dialog restores:
                // the card area must have it for the keys to work.
                move || page.park_focus()
            ),
        );
    }

    /// Shows the information of the card on screen. The session carries on
    /// as it was: nothing is graded, and the timer keeps running (the time
    /// of an answer is capped, ADR 0014).
    fn card_info(&self) {
        let imp = self.imp();
        let (Some(state), Some(id)) = (imp.state.get(), imp.current.borrow().clone()) else {
            return;
        };
        if imp.editing.get() {
            return;
        }
        card_info::open(
            self,
            state,
            &id,
            glib::clone!(
                #[weak(rename_to = page)]
                self,
                // The card area needs the focus for the keys to work.
                move || page.park_focus()
            ),
        );
    }

    /// R: plays the card's audio from its start ([`replay_target`]).
    fn replay_audio(&self) {
        let imp = self.imp();
        let front = audio_controls(&*imp.front_box);
        // `get_visible`, the back's own flag: `is_visible` also asks the
        // parents (`gtk_widget_is_visible`).
        let back = if imp.back_part.get_visible() {
            audio_controls(&*imp.back_box)
        } else {
            Vec::new()
        };
        let available = |controls: &[crate::render::CardAudio]| {
            controls
                .iter()
                .map(crate::render::CardAudio::is_available)
                .collect::<Vec<_>>()
        };
        let target = replay_target(imp.revealed.get(), &available(&front), &available(&back));
        let control = match target {
            Some((Side::Front, i)) => front.get(i),
            Some((Side::Back, i)) => back.get(i),
            None => None,
        };
        if let Some(control) = control {
            control.play_from_start();
        }
    }

    /// Takes the card on screen out of the session by suspending or deleting
    /// it, shows the next card, and offers Undo in a toast (ADR 0037). The
    /// card is not counted as answered.
    fn remove_current(&self, removal: Removal) {
        let imp = self.imp();
        if imp.editing.get() || imp.asking.get() {
            return;
        }
        let Some(id) = imp.current.borrow().clone() else {
            return;
        };
        self.remove(&id, removal);
    }

    fn remove(&self, id: &str, removal: Removal) {
        let imp = self.imp();
        let Some(state) = imp.state.get() else {
            return;
        };
        let written = match removal {
            Removal::Suspend => state
                .set_suspended(&[id.to_owned()], true)
                .map(|changed| !changed.is_empty()),
            Removal::Delete => state.delete_card(id).map(|()| true),
        };
        match written {
            Ok(true) => {}
            Ok(false) => {
                // Suspended elsewhere already: there is nothing to undo here.
                self.leave_session(id);
                show_toast_from(self, info_toast(&gettext("The card was already suspended")));
                return;
            }
            Err(rappel_core::Error::CardNotFound(_)) => {
                // Deleted elsewhere meanwhile: it is gone from the session too.
                self.leave_session(id);
                show_toast_from(self, info_toast(&gettext("The card was already gone")));
                return;
            }
            Err(err) => {
                // Translators: a toast: what is before ":" is its title, the rest is behind Details. {error} is why, in English.
                self.error_toast(&with_error(
                    &gettext("The card could not be changed: {error}"),
                    &err,
                ));
                return;
            }
        }
        self.leave_session(id);

        // Undo must work after leaving the page, so the closure keeps the
        // state and weak references, not the page.
        let window = self.root().and_downcast::<RappelWindow>();
        if let Some(window) = &window {
            window.data_changed();
        }
        let (id, state) = (id.to_owned(), state.clone());
        let page = self.downgrade();
        let window = window.map(|w| w.downgrade());
        let toast = undo_toast(&removal.text(), move || {
            undo_removal(&state, &id, removal, &page, window.as_ref());
        });
        show_toast_from(self, toast);
    }

    /// Drops card `id` from the session and, if it is on screen, shows the
    /// next one.
    fn leave_session(&self, id: &str) {
        let imp = self.imp();
        if let Some(session) = imp.session.borrow_mut().as_mut() {
            session.skip(id);
        }
        if imp.current.borrow().as_deref() == Some(id) {
            imp.current.replace(None);
            self.show_next();
        }
    }

    /// Move to Deck… (Card Menu): asks for a deck other than the card's,
    /// moves the card there, and takes it out of the session as Suspend
    /// does (ADR 0037, 0064), with an Undo toast. Nothing is counted as
    /// answered.
    fn move_current(&self) {
        let imp = self.imp();
        if imp.editing.get() || imp.asking.get() || !imp.can_move.get() {
            return;
        }
        let (Some(id), Some(state)) = (imp.current.borrow().clone(), imp.state.get().cloned())
        else {
            return;
        };
        let targets = match state.card(&id).and_then(|card| {
            let decks = state.decks()?;
            Ok(move_card::targets(&decks, &card.deck_id)
                .into_iter()
                .cloned()
                .collect::<Vec<Deck>>())
        }) {
            Ok(targets) => targets,
            Err(rappel_core::Error::CardNotFound(_)) => {
                // Deleted elsewhere meanwhile: it is gone from the session too.
                self.leave_session(&id);
                show_toast_from(self, info_toast(&gettext("The card was already gone")));
                return;
            }
            Err(err) => {
                // Translators: a toast: what is before ":" is its title, the rest is behind Details. {error} is why, in English.
                self.error_toast(&with_error(
                    &gettext("The card could not be changed: {error}"),
                    &err,
                ));
                return;
            }
        };
        if targets.is_empty() {
            // The last other deck went since the card was shown.
            imp.can_move.set(false);
            self.action_set_enabled("review.move", false);
            show_toast_from(self, info_toast(&move_card::no_other_deck_text()));
            return;
        }
        imp.asking.set(true);
        glib::spawn_future_local(glib::clone!(
            #[weak(rename_to = page)]
            self,
            async move {
                // AdwDialog gives the focus back to the widget that had it
                // when it has closed (see `confirm_delete`): the card area
                // needs it for the keys to work.
                let closed = glib::clone!(
                    #[weak]
                    page,
                    move || page.park_focus()
                );
                let answer = move_card::choose_deck(&page, targets, closed).await;
                page.imp().asking.set(false);
                // Only the card the question was about.
                if let Some(target) = answer
                    && page.imp().current.borrow().as_deref() == Some(&id)
                {
                    page.finish_move(&id, &target);
                }
            }
        ));
    }

    /// Moves card `id` to `target`, then the card leaves the session.
    fn finish_move(&self, id: &str, target: &Deck) {
        let Some(state) = self.imp().state.get() else {
            return;
        };
        let moved = match state.move_cards(&[id.to_owned()], &target.id) {
            Ok(moved) if !moved.is_empty() => moved,
            Ok(_) => {
                // In that deck already: it is not this session's any more.
                self.leave_session(id);
                show_toast_from(self, info_toast(&gettext("The card was already moved")));
                return;
            }
            Err(rappel_core::Error::CardNotFound(_)) => {
                self.leave_session(id);
                show_toast_from(self, info_toast(&gettext("The card was already gone")));
                return;
            }
            Err(rappel_core::Error::DeckNotFound(_)) => {
                // The deck was deleted while the choice was open.
                show_toast_from(self, info_toast(&crate::ui::selection::text::stale_text()));
                return;
            }
            Err(err) => {
                // Translators: a toast: what is before ":" is its title, the rest is behind Details. {error} is why, in English.
                self.error_toast(&with_error(
                    &gettext("The card could not be moved: {error}"),
                    &err,
                ));
                return;
            }
        };
        self.leave_session(id);

        // As for Suspend: Undo must work after leaving the page, so the
        // closure keeps the state and weak references, not the page.
        let window = self.root().and_downcast::<RappelWindow>();
        if let Some(window) = &window {
            window.data_changed();
        }
        let (id, state) = (id.to_owned(), state.clone());
        let page = self.downgrade();
        let window = window.map(|w| w.downgrade());
        let toast = move_card::moved_toast(&state.clone(), moved, move |result| {
            undo_move(&state, &id, result, &page, window.as_ref());
        });
        show_toast_from(self, toast);
    }

    /// Asks before deleting the card on screen. Cancel is the default.
    fn confirm_delete(&self) {
        let imp = self.imp();
        if imp.editing.get() || imp.asking.get() {
            return;
        }
        let Some(id) = imp.current.borrow().clone() else {
            return;
        };
        let Ok(question) = dialog::DELETE_CARD.load() else {
            return;
        };
        imp.asking.set(true);
        // AdwDialog gives the focus back to the widget that had it when it
        // has closed, which would undo a grab made on the response (the
        // editor and Card Information set theirs on `closed` for the same
        // reason): the card area needs the focus for the keys to work.
        question.connect_closed(glib::clone!(
            #[weak(rename_to = page)]
            self,
            move |_| page.park_focus()
        ));
        glib::spawn_future_local(glib::clone!(
            #[weak(rename_to = page)]
            self,
            async move {
                let answer = question.choose_future(Some(&page)).await;
                page.imp().asking.set(false);
                // Only the card the question was about.
                if answer == "delete" && page.imp().current.borrow().as_deref() == Some(&id) {
                    page.remove(&id, Removal::Delete);
                }
            }
        ));
    }

    /// Brings card `id` back into the session after the Undo of a suspend or
    /// a delete: it comes next (a card an undo brought back, or a step card
    /// that is due, still goes first), and the summary gives way to it.
    fn put_back(&self, id: &str) {
        let imp = self.imp();
        // A card of a deck archived since does not come back (ADR 0069).
        if let Some(state) = imp.state.get()
            && state
                .card(id)
                .is_ok_and(|card| imp.left_decks.borrow().contains(&card.deck_id))
        {
            return;
        }
        if let Some(session) = imp.session.borrow_mut().as_mut() {
            session.put_back(id);
        }
        if imp.current.borrow().is_none() {
            self.show_next();
        } else {
            let left = imp.session.borrow().as_ref().map_or(0, Run::remaining);
            imp.window_title.set_subtitle(&left_text(left));
        }
    }

    /// A save made from the editor: shows the card on screen again if it
    /// is the one saved, then tells the owner of this page.
    fn saved(&self, saved: Saved) {
        let imp = self.imp();
        if let Saved::Edited(tagged) = &saved
            && imp.current.borrow().as_deref() == Some(tagged.card.id.as_str())
        {
            self.redraw(&tagged.card.front_md, &tagged.card.back_md);
        }
        if let Some(on_saved) = imp.on_saved.get().map(Callback::get) {
            on_saved(saved);
        }
    }

    /// Renders the card on screen from new texts. Only what is shown
    /// changes: the reveal, the timers and the session stay as they are.
    fn redraw(&self, front_md: &str, back_md: &str) {
        let imp = self.imp();
        self.show_front(front_md);
        self.keep_back(back_md);
        if imp.revealed.get() {
            let back = imp.back.take();
            // A redraw never animates: the answer was already on screen,
            // so it changes in place (only `reveal` plays the animation).
            let _shown = self.show_back(&back);
        }
    }

    /// Shows `controls` in the bottom bar and enables only their actions.
    fn set_controls(&self, controls: Controls) {
        let (name, reveal, grade) = match controls {
            Controls::Reveal => (Some("reveal"), true, false),
            Controls::Grade => (Some("grade"), false, true),
            Controls::Hidden => (None, false, false),
        };
        // Hidden with the summary, where there is no card to edit.
        let editable = !matches!(controls, Controls::Hidden);
        self.action_set_enabled("review.edit", editable);
        self.action_set_enabled("review.card-info", editable);
        self.action_set_enabled("review.suspend", editable);
        self.action_set_enabled("review.move", editable && self.imp().can_move.get());
        self.action_set_enabled("review.delete", editable);
        self.imp().editable.set(editable);
        self.show_header_buttons();
        // The summary has its own button; the bar goes, and the summary
        // gets its room.
        self.imp().bottom_bar.set_visible(name.is_some());
        if let Some(name) = name {
            self.imp().buttons.set_visible_child_name(name);
        }
        self.action_set_enabled("review.reveal", reveal);
        self.action_set_enabled("review.fail", grade);
        self.action_set_enabled("review.pass", grade);
        self.update_undo();
    }

    /// Enables Undo while the session has an answer to take back.
    fn update_undo(&self) {
        let can_undo = self
            .imp()
            .session
            .borrow()
            .as_ref()
            .is_some_and(Run::can_undo);
        self.action_set_enabled("review.undo", can_undo);
    }

    /// Takes back the last answer of the session: the log first, then the
    /// session, then the card is shown again as new (front side, timer
    /// restarted), whether a card or the summary was on screen.
    fn undo(&self) {
        if self.is_practice() {
            self.undo_practice();
            return;
        }
        let imp = self.imp();
        let Some(state) = imp.state.get() else {
            return;
        };
        let last = match &*imp.session.borrow() {
            Some(Run::Study(session)) => session
                .last_answer()
                .map(|(c, e)| (c.to_owned(), e.to_owned())),
            _ => None,
        };
        let Some((card_id, entry_id)) = last else {
            return;
        };
        match state.undo(&card_id, &entry_id, rappel_core::time::now_ms()) {
            Ok(_) => {
                if let Some(Run::Study(session)) = imp.session.borrow_mut().as_mut() {
                    session.undo_last();
                }
                imp.current.replace(None);
                self.show_next();
            }
            Err(err) => match undo::failure(&err) {
                UndoFailure::Stale => {
                    // Older answers are undone only after this one, so none
                    // of them can be now.
                    if let Some(Run::Study(session)) = imp.session.borrow_mut().as_mut() {
                        session.forget_answers();
                    }
                    self.update_undo();
                    // The button may have had the focus and is now off:
                    // put it where the page's keys work.
                    self.park_focus();
                    show_toast_from(
                        self,
                        info_toast(&gettext("This answer can no longer be undone")),
                    );
                }
                UndoFailure::Failed => {
                    // Translators: a toast: what is before ":" is its title, the rest is behind Details. {error} is why, in English.
                    self.error_toast(&with_error(
                        &gettext("The answer could not be undone: {error}"),
                        &err,
                    ));
                }
            },
        }
    }

    /// Records the answer for the card on screen and shows the next one.
    /// If it cannot be recorded, the card stays so the answer can be given
    /// again.
    fn grade(&self, grade: Grade) {
        if self.is_practice() {
            self.grade_practice(grade);
            return;
        }
        let imp = self.imp();
        let Some(state) = imp.state.get() else {
            return;
        };
        if !imp.revealed.get() {
            return;
        }
        let Some(id) = imp.current.borrow().clone() else {
            return;
        };
        let duration = capped(imp.shown_at.get().map_or(Duration::ZERO, |t| t.elapsed()));
        let now = rappel_core::time::now_ms();
        let event = match state.record_review(&id, grade, duration, now) {
            Ok(event) => event,
            Err(err) => {
                // Translators: a toast: what is before ":" is its title, the rest is behind Details. {error} is why, in English.
                self.error_toast(&with_error(
                    &gettext("The answer could not be saved: {error}"),
                    &err,
                ));
                return;
            }
        };
        let card_state = state.card_state(&id).unwrap_or_else(|err| {
            // The answer is saved; without its new state the card just
            // does not come back in this session.
            tracing::warn!("cannot read a card's state after an answer: {err}");
            CardState::New
        });
        if let Some(Run::Study(session)) = imp.session.borrow_mut().as_mut() {
            session.graded(&id, &event.id, grade, duration, &card_state);
        }
        imp.current.replace(None);
        self.show_next();
    }

    /// A practice's answer for the card on screen (ADR 0070): kept in the
    /// [`Practice`] alone, which shows the next card. Nothing is written:
    /// this does not use the [`AppState`] (the practice tests check the
    /// database is unchanged).
    fn grade_practice(&self, grade: Grade) {
        let imp = self.imp();
        if !imp.revealed.get() {
            return;
        }
        let Some(id) = imp.current.borrow().clone() else {
            return;
        };
        let duration = imp.shown_at.get().map_or(Duration::ZERO, |t| t.elapsed());
        if let Some(Run::Practice(practice)) = imp.session.borrow_mut().as_mut() {
            practice.graded(&id, grade, duration);
        }
        imp.current.replace(None);
        self.show_next();
    }

    /// Steps a practice back one answer (ADR 0070): the card is shown
    /// again, front first, and the summary loses the answer. Only memory
    /// changes, as for [`Self::grade_practice`].
    ///
    /// An answer whose card can no longer be practised (deleted, suspended,
    /// moved out of a tag's decks by an archive, `practisable_card`) is
    /// taken back silently, its card left out, and the step goes on to the
    /// answer before it: one Ctrl+Z always shows a card that came back,
    /// never an error about one that is gone. With no such answer left,
    /// the card on screen stays as it was.
    fn undo_practice(&self) {
        let imp = self.imp();
        let back = loop {
            let undone = match imp.session.borrow_mut().as_mut() {
                Some(Run::Practice(practice)) => practice.undo_last(),
                _ => None,
            };
            let Some(id) = undone else {
                break None;
            };
            if self.practisable_card(&id) {
                break Some(id);
            }
            if let Some(Run::Practice(practice)) = imp.session.borrow_mut().as_mut() {
                practice.skip(&id);
            }
        };
        if back.is_some() || imp.current.borrow().is_none() {
            // A card came back, or the summary is read again.
            imp.current.replace(None);
            self.show_next();
        } else {
            self.update_undo();
            let left = imp.session.borrow().as_ref().map_or(0, Run::remaining);
            imp.window_title.set_subtitle(&left_text(left));
        }
    }

    /// Whether card `id` can still be practised here: live, not
    /// suspended, in a deck that is not archived nor left by this page
    /// (`left_decks`), and still in the practice's scope: in its deck (not
    /// moved since), or carrying its tag (not untagged since). Only reads.
    fn practisable_card(&self, id: &str) -> bool {
        let imp = self.imp();
        let (Some(state), Some(studied)) = (imp.state.get(), imp.studied.get()) else {
            return false;
        };
        let Ok(card) = state.card(id) else {
            return false;
        };
        let in_scope = match studied {
            Studied::Deck { deck, .. } => card.deck_id == deck.id,
            Studied::Tag { tag, .. } => state
                .card_tags(id)
                .is_ok_and(|tags| tags.iter().any(|t| t.id == tag.id)),
        };
        in_scope
            && !card.suspended
            && !imp.left_decks.borrow().contains(&card.deck_id)
            && state
                .deck(&card.deck_id)
                .is_ok_and(|deck| !deck.is_archived())
    }

    /// Practice from the summary's button: this page gives way to a
    /// practice of the same deck or tag, so Back from it goes where Back
    /// from here went (`nav::replace_visible`).
    fn practise_instead(&self) {
        let imp = self.imp();
        let (Some(state), Some(studied)) = (imp.state.get(), imp.studied.get()) else {
            return;
        };
        let Some(view) = nav::navigation_view(self) else {
            return;
        };
        let on_saved = imp.on_saved.get().map(Callback::get);
        let started = Self::practice(state.clone(), studied.clone(), move |saved| {
            if let Some(on_saved) = &on_saved {
                on_saved(saved);
            }
        });
        match started {
            Ok(page) => nav::replace_visible(&view, &page),
            Err(err) => {
                // Translators: a toast: what is before ":" is its title, the rest is behind Details. {error} is why, in English.
                self.error_toast(&with_error(
                    &gettext("The session could not start: {error}"),
                    &err,
                ));
            }
        }
    }

    fn show_summary(&self) {
        let imp = self.imp();
        imp.current.replace(None);
        imp.shown_at.set(None);
        let summary = imp
            .session
            .borrow()
            .as_ref()
            .map(Run::summary)
            .unwrap_or_default();
        let practising = self.is_practice();
        let tiles = summary_tiles(&summary, practising);
        // Read only after a session: a practice never offers it.
        let practisable = !practising && self.practisable();
        imp.practice_button
            .set_visible(practice_button_shown(practising, practisable));
        // A practice's summary says what it did in words, and gives the
        // cards practised and the pass rate: the banner and the time would
        // only repeat or crowd it (a phone shows it all without scrolling).
        imp.banner.set_revealed(false);
        imp.time_tile.set_visible(!practising);
        if practising {
            // The description is Pango markup; this text is ours, but
            // escaped all the same, as every text put there.
            imp.summary_figures.set_visible(tiles.is_some());
            if let Some([cards, passed, time]) = tiles {
                set_tile(&imp.cards_value, &imp.cards_caption, &cards);
                set_tile(&imp.passed_value, &imp.passed_caption, &passed);
                set_tile(&imp.time_value, &imp.time_caption, &time);
            }
            imp.summary_page.set_title(&practice_complete());
            imp.summary_page
                .set_description(Some(&glib::markup_escape_text(&nothing_rescheduled())));
        } else if let Some([cards, passed, time]) = tiles {
            set_tile(&imp.cards_value, &imp.cards_caption, &cards);
            set_tile(&imp.passed_value, &imp.passed_caption, &passed);
            set_tile(&imp.time_value, &imp.time_caption, &time);
            imp.summary_figures.set_visible(true);
            imp.summary_page.set_title(&session_complete());
            imp.summary_page.set_description(None);
        } else if imp.started_empty.get()
            && let Some(studied) = imp.studied.get()
        {
            // Nothing was there to study: the "all done" page.
            imp.summary_figures.set_visible(false);
            imp.summary_page.set_title(&nothing_to_study());
            // The description is Pango markup (`AdwStatusPage:description`)
            // and the text names a deck or a tag, which is untrusted.
            imp.summary_page
                .set_description(Some(&glib::markup_escape_text(&studied.nothing_to_study())));
        } else {
            imp.summary_figures.set_visible(false);
            imp.summary_page.set_title(&session_complete());
            imp.summary_page
                .set_description(Some(&glib::markup_escape_text(&nothing_reviewed())));
        }
        imp.window_title.set_subtitle("");
        self.set_controls(Controls::Hidden);
        imp.stack.set_visible_child_name("summary");
        imp.done_button.grab_focus();
    }

    fn done(&self) {
        nav::pop(self);
    }

    /// Whether a practice of what this page studies would show a card
    /// (`AppState::has_practisable`; `false` if that cannot be read: the
    /// button is then not offered).
    fn practisable(&self) -> bool {
        let imp = self.imp();
        let (Some(state), Some(studied)) = (imp.state.get(), imp.studied.get()) else {
            return false;
        };
        state
            .has_practisable(&studied.scope())
            .unwrap_or_else(|err| {
                tracing::warn!("cannot read what to practise: {err}");
                false
            })
    }

    /// Shows `message`, dismissing the previous error toast of this page
    /// first, so a toast that stays until dismissed does not hold it back.
    fn error_toast(&self, message: &str) {
        self.imp().errors.show(self, "a review operation", message);
    }
}

/// Whether the note "only its start is shown" is under the card: the front
/// is cut, or the back is cut and shown.
fn cut_note_shown(front_cut: bool, back_cut: bool, revealed: bool) -> bool {
    front_cut || (back_cut && revealed)
}

/// What screen readers read for a side: its text, and the note when only
/// its start is shown, since the note itself is not in the focus chain.
fn with_cut_note(text: String, cut: bool) -> String {
    if cut {
        format!("{text}\n\n{}", cut_note())
    } else {
        text
    }
}

/// The note's text for screen readers. Keep it equal to `cut_note` in
/// `review_page.blp`.
fn cut_note() -> String {
    gettext("This card is long: only its start is shown. Edit it to see all of it.")
}

/// The window width under which Edit Card leaves the header for the Card
/// Menu: at 360 px with minimise and maximise shown, the title and the
/// subtitle ("30 restantes") of a session are cut beside Back, Undo, Edit,
/// the menu and the window buttons.
const EDIT_IN_MENU_WIDTH: i32 = 400;

/// Whether the header shows its Edit Card button: with a card on screen
/// (`editable`), unless the window is `narrow`, where the Card Menu has it.
fn edit_button_shown(editable: bool, narrow: bool) -> bool {
    editable && !narrow
}

/// The subtitle while studying: "12 left", "1,234 left". It counts the
/// card on screen and step cards that will come back, so it can go up
/// after a Fail.
fn left_text(left: usize) -> String {
    // Translators: the subtitle while studying; {count} is the number of
    // cards left in the session.
    counted(&ngettext("{count} left", "{count} left", left), left)
}

/// How long the answer takes to come in, in milliseconds: short enough
/// never to hold anyone up.
const REVEAL_MS: u32 = 180;

/// How far below its place the answer starts, in pixels.
const REVEAL_SLIDE: f64 = 12.0;

/// The back's opacity and top margin at `progress` (0 to 1, clamped) of the
/// reveal: from hidden and [`REVEAL_SLIDE`] lower to fully shown in place.
fn reveal_step(progress: f64) -> (f64, i32) {
    let progress = if progress.is_nan() {
        1.0
    } else {
        progress.clamp(0.0, 1.0)
    };
    #[expect(
        clippy::cast_possible_truncation,
        reason = "a value between 0 and REVEAL_SLIDE, rounded"
    )]
    let margin = (REVEAL_SLIDE * (1.0 - progress)).round() as i32;
    (progress, margin)
}

/// The least of the answer that must be in view after the reveal, in
/// pixels: a few lines of text.
const ANSWER_SHOWN: f64 = 96.0;

/// Room left above the answer when the card scrolls to it, in pixels.
const ANSWER_MARGIN: f64 = 12.0;

/// How far to scroll the card at the reveal, for an answer starting
/// `back_top` pixels below the top of a view `page` pixels tall: none if
/// at least [`ANSWER_SHOWN`] of it is in view already (or the view is not
/// laid out), else so that it starts [`ANSWER_MARGIN`] below the top. The
/// scrolled window clamps a scroll past its end.
fn answer_scroll(back_top: f64, page: f64) -> f64 {
    if page <= 0.0 || back_top + ANSWER_SHOWN.min(page / 2.0) <= page {
        0.0
    } else {
        back_top - ANSWER_MARGIN
    }
}

/// The summary's icon, a check mark. Keep it equal to `summary_page` in
/// `review_page.blp`.
const SUMMARY_ICON: &str = "object-select-symbolic";

/// The summary page's text when no card was answered (all of them were
/// suspended or deleted during the session).
fn nothing_reviewed() -> String {
    gettext("No cards were reviewed")
}

/// The summary's title after a session. Keep it equal to `summary_page`
/// in `review_page.blp`.
fn session_complete() -> String {
    gettext("Session Complete")
}

/// The summary's title when there was nothing to study at the start (the
/// "all done" page).
fn nothing_to_study() -> String {
    // Translators: the title of the review page when a deck or a tag has
    // no card to study now.
    gettext("Nothing to Study")
}

/// Where a session was started: its summary's button goes back there
/// (the header's Back always does).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StartedFrom {
    /// A deck's page (Study, a card's tag in its menu, Practice in the
    /// Deck Menu).
    DeckPage,
    /// All Cards (its tag filter, or a card's tag in its menu).
    AllCards,
    /// The deck list (Practice in a deck's ⋮ menu, ADR 0070).
    DeckList,
}

impl StartedFrom {
    /// The summary's button, which leaves the page.
    fn done_label(self) -> String {
        match self {
            Self::DeckPage => gettext("_Back to Deck"),
            Self::AllCards => gettext("_Back to All Cards"),
            Self::DeckList => gettext("_Back to Decks"),
        }
    }
}

/// Whether a session studies or practises (ADR 0070).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Mode {
    /// A session: answers are recorded and schedule the cards.
    Study,
    /// A practice: nothing is written.
    Practice,
}

/// What the page runs: a study session, or a practice. The page asks it
/// the card to show and what is left; answers and their Undo go to each
/// in its own way (`grade`, `grade_practice`).
#[derive(Debug)]
pub(super) enum Run {
    /// A session: each answer is recorded with `record_review`.
    Study(Session),
    /// A practice: answers are kept in memory only (ADR 0070).
    Practice(Practice),
}

impl Run {
    fn next(&self, now: i64) -> Option<&str> {
        match self {
            Self::Study(session) => session.next(now),
            Self::Practice(practice) => practice.next(),
        }
    }

    fn remaining(&self) -> usize {
        match self {
            Self::Study(session) => session.remaining(),
            Self::Practice(practice) => practice.remaining(),
        }
    }

    fn skip(&mut self, card_id: &str) {
        match self {
            Self::Study(session) => session.skip(card_id),
            Self::Practice(practice) => practice.skip(card_id),
        }
    }

    fn put_back(&mut self, card_id: &str) {
        match self {
            Self::Study(session) => session.put_back(card_id),
            Self::Practice(practice) => practice.put_back(card_id),
        }
    }

    fn can_undo(&self) -> bool {
        match self {
            Self::Study(session) => session.last_answer().is_some(),
            Self::Practice(practice) => practice.can_undo(),
        }
    }

    fn summary(&self) -> Summary {
        match self {
            Self::Study(session) => session.summary(),
            Self::Practice(practice) => practice.summary(),
        }
    }
}

/// Whether Practice is offered for a deck or a tag with `practisable`
/// cards to practise (live, not suspended, in a deck that is not
/// archived): only when there is at least one, so it never opens on
/// nothing. The Deck Menu, the deck rows' menus and the summary ask it.
pub fn practice_offered(practisable: usize) -> bool {
    practisable > 0
}

/// Whether the summary shows its Practice button: after a session, not a
/// practice (which would only start over), when the scope has a card to
/// practise (`practisable`, from `AppState::has_practisable`).
fn practice_button_shown(practising: bool, practisable: bool) -> bool {
    !practising && practisable
}

/// The practice summary's title. Keep `practice_complete` and
/// [`nothing_rescheduled`] as the brief words them (ADR 0070).
fn practice_complete() -> String {
    gettext("Practice Complete")
}

/// What the practice summary says, under its title.
fn nothing_rescheduled() -> String {
    gettext("Practice: nothing was rescheduled")
}

/// What a session studies (ADR 0069).
#[derive(Debug, Clone)]
pub enum Studied {
    /// One deck.
    Deck {
        deck: Deck,
        /// Where the session was started.
        from: StartedFrom,
    },
    /// The cards carrying `tag` in every deck that is not archived.
    Tag {
        tag: Tag,
        /// Where the session was started.
        from: StartedFrom,
    },
}

impl Studied {
    /// The core's scope of the session.
    fn scope(&self) -> StudyScope {
        match self {
            Self::Deck { deck, .. } => StudyScope::deck(&deck.id),
            Self::Tag { tag, .. } => StudyScope::tag(&tag.id),
        }
    }

    /// The page's title: the deck's name, or "Tagged “verbs”".
    fn title(&self) -> String {
        match self {
            Self::Deck { deck, .. } => deck.name.clone(),
            // Translators: the title of the review page while studying the
            // cards that carry a tag, in every deck; {tag} is its name.
            Self::Tag { tag, .. } => i18n::fill(
                &gettext("Tagged “{tag}”"),
                &[("tag", &isolate(&one_line(&tag.name)))],
            ),
        }
    }

    /// The summary's button, which leaves the page.
    fn done_label(&self) -> String {
        match self {
            Self::Deck { from, .. } | Self::Tag { from, .. } => from.done_label(),
        }
    }

    /// The "all done" page's text: there was nothing to study.
    fn nothing_to_study(&self) -> String {
        match self {
            // Translators: under "Nothing to Study"; {deck} is a deck's name.
            Self::Deck { deck, .. } => i18n::fill(
                &gettext("No cards of “{deck}” are left to study now."),
                &[("deck", &isolate(&one_line(&deck.name)))],
            ),
            // Translators: under "Nothing to Study"; {tag} is a tag's name.
            Self::Tag { tag, .. } => i18n::fill(
                &gettext("No cards tagged “{tag}” are left to study now."),
                &[("tag", &isolate(&one_line(&tag.name)))],
            ),
        }
    }
}

/// The summary's three tiles: cards reviewed (or practised), pass rate and
/// time, or `None` when no card was answered.
fn summary_tiles(summary: &Summary, practising: bool) -> Option<[Tile; 3]> {
    let percent = summary.pass_percent().filter(|_| summary.cards > 0)?;
    let cards_caption = if practising {
        // Translators: the caption under the number of cards practised in
        // a practice's summary (nothing was rescheduled).
        ngettext("Card practised", "Cards practised", summary.cards)
    } else {
        // Translators: the caption under the number of cards reviewed in a
        // session's summary.
        ngettext("Card reviewed", "Cards reviewed", summary.cards)
    };
    let tile = |value: String, caption: String| Tile {
        // Translators: what a screen reader says for a headline figure:
        // {caption} is "Pass rate", {value} "91%".
        accessible: i18n::fill(
            &gettext("{caption}: {value}"),
            &[("caption", &caption), ("value", &value)],
        ),
        value,
        caption,
    };
    Some([
        tile(group_digits(summary.cards), cards_caption),
        tile(whole_percent(percent), gettext("Pass rate")),
        tile(duration_text(summary.time), gettext("Time")),
    ])
}

#[cfg(test)]
mod tests {
    use rappel_core::domain::EventKind;

    use super::*;
    use crate::render::MAX_SHOWN_BYTES;

    #[test]
    fn edit_card_leaves_the_header_of_a_narrow_window() {
        assert!(edit_button_shown(true, false));
        assert!(!edit_button_shown(true, true));
        // No card on screen (the summary): no button at all.
        assert!(!edit_button_shown(false, false));
        assert!(!edit_button_shown(false, true));
    }

    #[test]
    fn the_cut_note_follows_the_sides_on_screen() {
        assert!(!cut_note_shown(false, false, false));
        assert!(!cut_note_shown(false, false, true));
        assert!(cut_note_shown(true, false, false));
        // A cut back is only noted once it is shown.
        assert!(!cut_note_shown(false, true, false));
        assert!(cut_note_shown(false, true, true));
    }

    #[test]
    fn screen_readers_hear_the_cut_note() {
        assert_eq!(with_cut_note("Front".to_owned(), false), "Front");
        assert_eq!(
            with_cut_note("Front".to_owned(), true),
            format!("Front\n\n{}", cut_note())
        );
    }

    #[test]
    fn screen_readers_get_the_card_from_the_card_area_and_the_note_once() {
        crate::testing::with_widgets(|| {
            let page: ReviewPage = glib::Object::new();
            let imp = page.imp();
            // A generic role (a box's default) has no name or description
            // for screen readers; a group has both.
            assert_eq!(imp.card_area.accessible_role(), gtk::AccessibleRole::Group);
            assert!(imp.card_area.is_focusable());
            // The note is in the card area's name or description
            // (`with_cut_note`); the label itself is not exposed.
            assert_eq!(
                imp.cut_note.accessible_role(),
                gtk::AccessibleRole::Presentation
            );
            page.show_front(&"word ".repeat(9_000));
            assert!(imp.cut_note.is_visible());
            // Only that a label is set: GTK has no public getter for its
            // value (`gtk_test_accessible_check_property` is only in
            // gtk4-sys, variadic and unsafe), so the text with the note is
            // checked in `with_cut_note`'s tests and was read over AT-SPI
            // (ADR 0051).
            assert!(gtk::test_accessible_has_property(
                &*imp.card_area,
                gtk::AccessibleProperty::Label
            ));
        });
    }

    #[test]
    fn the_cut_note_matches_the_blueprint() {
        crate::testing::with_widgets(|| {
            let page: ReviewPage = glib::Object::new();
            assert_eq!(page.imp().cut_note.label(), cut_note());
        });
    }

    #[test]
    fn keys_reveal_grade_and_edit() {
        use Key::{Edit, Enter, Fail, Pass, Space};
        let grade = |g| Some(Act::Grade(g));
        // Before the answer.
        assert_eq!(key_action(Space, false, false), Some(Act::Reveal));
        assert_eq!(key_action(Enter, false, false), Some(Act::Reveal));
        assert_eq!(key_action(Fail, false, true), None);
        assert_eq!(key_action(Pass, false, true), None);
        // After it, once settled.
        assert_eq!(key_action(Space, true, true), grade(Grade::Pass));
        assert_eq!(key_action(Pass, true, true), grade(Grade::Pass));
        assert_eq!(key_action(Fail, true, true), grade(Grade::Fail));
        assert_eq!(key_action(Enter, true, true), None);
        // Key repeat right after the reveal grades nothing.
        for key in [Space, Pass, Fail] {
            assert_eq!(key_action(key, true, false), None);
        }
        // E edits at any time, settled or not. So does I, to look.
        for (revealed, settled) in [(false, false), (true, false), (true, true)] {
            assert_eq!(key_action(Edit, revealed, settled), Some(Act::Edit));
            assert_eq!(key_action(Key::Info, revealed, settled), Some(Act::Info));
        }
        // S and Delete too: they grade nothing.
        for (revealed, settled) in [(false, false), (true, false), (true, true)] {
            assert_eq!(
                key_action(Key::Suspend, revealed, settled),
                Some(Act::Suspend)
            );
            assert_eq!(
                key_action(Key::Delete, revealed, settled),
                Some(Act::Delete)
            );
        }
        // Ctrl+Z too, and it grades nothing, so key repeat does no harm.
        for (revealed, settled) in [(false, false), (true, false), (true, true)] {
            assert_eq!(key_action(Key::Undo, revealed, settled), Some(Act::Undo));
        }
    }

    /// A page over a deck of `n` cards that are all due, with the app state.
    fn page_of(n: usize) -> (tempfile::TempDir, AppState, ReviewPage, Vec<String>) {
        let sides: Vec<(String, String)> = (0..n)
            .map(|i| (format!("front {i}"), "back".to_owned()))
            .collect();
        page_with(&sides)
    }

    /// A page over a deck of one card per (front, back) of `sides`, all due
    /// now, with the app state and the card ids. The one fixture of these
    /// tests (#57).
    fn page_with(
        sides: &[(String, String)],
    ) -> (tempfile::TempDir, AppState, ReviewPage, Vec<String>) {
        let (tmp, state) = crate::testing::temporary_state();
        let (page, ids) = page_over(&state, sides);
        (tmp, state, page, ids)
    }

    /// [`page_with`] over the database of `state`.
    fn page_over(state: &AppState, sides: &[(String, String)]) -> (ReviewPage, Vec<String>) {
        let deck = state.create_deck("Deck").unwrap();
        let now = rappel_core::time::now_ms();
        let day = crate::ui::time::DAY_MS;
        let ids: Vec<String> = sides
            .iter()
            .map(|(front, back)| {
                let id = state
                    .add_card_with_tags(&deck.id, front, back, &[])
                    .unwrap()
                    .card
                    .id;
                // Two Passes long ago leave the card in review and due now.
                for at in [now - 60 * day, now - 60 * day + 11 * 60 * 1000] {
                    state
                        .record_review(&id, Grade::Pass, Duration::ZERO, at)
                        .unwrap();
                }
                id
            })
            .collect();
        let page = ReviewPage::new(state.clone(), deck, |_| {}).unwrap();
        (page, ids)
    }

    /// Puts `page` in a window, so it has a root as a pushed page does.
    fn in_window(page: &ReviewPage) -> gtk::Window {
        gtk::Window::builder().child(page).build()
    }

    #[test]
    fn r_plays_the_back_once_shown_else_the_front() {
        use Side::{Back, Front};
        // Before the answer: the front's first that can play.
        assert_eq!(
            replay_target(false, &[true, true], &[true]),
            Some((Front, 0))
        );
        assert_eq!(
            replay_target(false, &[false, true], &[true]),
            Some((Front, 1))
        );
        assert_eq!(replay_target(false, &[], &[true]), None);
        // After it: the back's, when the back has audio.
        assert_eq!(
            replay_target(true, &[true], &[false, true]),
            Some((Back, 1))
        );
        assert_eq!(replay_target(true, &[true], &[]), Some((Front, 0)));
        // A side whose audio cannot play gives nothing, not the other's.
        assert_eq!(replay_target(true, &[true], &[false]), None);
        assert_eq!(replay_target(false, &[false], &[true]), None);
        assert_eq!(replay_target(true, &[], &[]), None);
        // R at any time, settled or not: it grades nothing.
        for (revealed, settled) in [(false, false), (true, false), (true, true)] {
            assert_eq!(
                key_action(Key::Replay, revealed, settled),
                Some(Act::Replay)
            );
        }
    }

    /// R reaches the right control through the page's key handler (the
    /// controls count the plays asked for, so no sound is needed).
    #[test]
    fn r_plays_the_audio_on_screen() {
        crate::testing::with_widgets(|| {
            let (tmp, state) = crate::testing::temporary_state();
            let data = tmp.path().join("data");
            let add = |samples| {
                AppState::add_media_bytes_on_worker(
                    &data,
                    &crate::testing::wav_bytes(samples),
                    crate::state::Accept::Audio,
                )
                .unwrap()
                .reference()
            };
            let (front_audio, back_audio) = (add(800), add(1_600));
            let sides = [
                (
                    format!("Hear [this]({front_audio})"),
                    format!("[Answer]({back_audio})"),
                ),
                (
                    format!("[Only the front]({front_audio})"),
                    "No audio".to_owned(),
                ),
            ];
            // The page is never on screen, so no stream is made: playback
            // may be taken as there whatever is installed.
            let _forced = crate::render::force_playback(true);
            let (page, ids) = page_over(&state, &sides);
            let _window = in_window(&page);
            let imp = page.imp();
            let plays = |container: &gtk::Box| {
                audio_controls(container)
                    .iter()
                    .map(crate::render::CardAudio::plays)
                    .collect::<Vec<_>>()
            };
            for _ in 0..2 {
                let with_back_audio = current(&page).as_deref() == Some(ids[0].as_str());
                assert_eq!(page.key(Key::Replay), glib::Propagation::Stop);
                assert_eq!(plays(&imp.front_box), [1]);
                page.reveal();
                page.key(Key::Replay);
                if with_back_audio {
                    // The back, once shown.
                    assert_eq!(plays(&imp.front_box), [1]);
                    assert_eq!(plays(&imp.back_box), [1]);
                } else {
                    // No audio on the back: the front again.
                    assert_eq!(plays(&imp.front_box), [2]);
                    assert_eq!(audio_controls(&*imp.back_box).len(), 0);
                }
                imp.revealed_at
                    .set(Instant::now().checked_sub(Duration::from_secs(1)));
                page.key(Key::Pass);
            }
            assert_eq!(current(&page), None);
        });
    }

    #[test]
    fn the_delete_question_loads_and_defaults_to_cancel() {
        crate::testing::with_widgets(|| {
            let question = dialog::DELETE_CARD.load().unwrap();
            assert_eq!(question.default_response().as_deref(), Some("cancel"));
            assert_eq!(question.close_response(), "cancel");
            assert_eq!(
                question.response_appearance("delete"),
                adw::ResponseAppearance::Destructive
            );
        });
    }

    #[test]
    fn s_waits_for_the_card_to_have_been_on_screen() {
        assert!(!suspend_allowed(None));
        assert!(!suspend_allowed(Some(Duration::ZERO)));
        assert!(!suspend_allowed(Some(
            KEY_SETTLE.saturating_sub(Duration::from_millis(1))
        )));
        assert!(suspend_allowed(Some(KEY_SETTLE)));
    }

    #[test]
    fn holding_s_suspends_one_card_not_a_run() {
        crate::testing::with_widgets(|| {
            let (_tmp, state, page, ids) = page_of(3);
            let _window = in_window(&page);
            let first = current(&page).unwrap();
            assert!(
                page.imp().shown_at.get().is_some(),
                "showing a card starts the settle time"
            );
            // Just shown: the key does nothing. Set again here, not only
            // when the window was made, so a slow machine cannot let 300 ms
            // pass in between (ADR 0049).
            page.imp().shown_at.set(Some(Instant::now()));
            page.key(Key::Suspend);
            assert_eq!(current(&page).as_deref(), Some(first.as_str()));
            assert!(!state.card(&first).unwrap().suspended);
            // Shown a moment: the first S suspends, the repeat finds a new card.
            page.imp()
                .shown_at
                .set(Instant::now().checked_sub(Duration::from_secs(1)));
            page.key(Key::Suspend);
            page.key(Key::Suspend);
            page.key(Key::Suspend);
            let suspended = ids
                .iter()
                .filter(|id| state.card(id).unwrap().suspended)
                .count();
            assert_eq!(suspended, 1);
        });
    }

    #[test]
    fn the_keys_do_nothing_while_the_editor_or_the_question_is_open() {
        crate::testing::with_widgets(|| {
            let (_tmp, state, page, _) = page_of(2);
            let _window = in_window(&page);
            let first = current(&page).unwrap();
            page.imp()
                .shown_at
                .set(Instant::now().checked_sub(Duration::from_secs(1)));
            page.imp().asking.set(true);
            assert_eq!(page.key(Key::Suspend), glib::Propagation::Proceed);
            assert_eq!(page.key(Key::Delete), glib::Propagation::Proceed);
            page.imp().asking.set(false);
            page.imp().editing.set(true);
            assert_eq!(page.key(Key::Suspend), glib::Propagation::Proceed);
            page.imp().editing.set(false);
            assert!(!state.card(&first).unwrap().suspended);
            assert_eq!(current(&page).as_deref(), Some(first.as_str()));
        });
    }

    #[test]
    fn a_card_already_suspended_elsewhere_leaves_the_session_without_an_undo() {
        crate::testing::with_widgets(|| {
            let (_tmp, state, page, _) = page_of(2);
            let _window = in_window(&page);
            let first = current(&page).unwrap();
            state
                .set_suspended(std::slice::from_ref(&first), true)
                .unwrap();
            page.remove_current(Removal::Suspend);
            assert_ne!(current(&page).as_deref(), Some(first.as_str()));
        });
    }

    fn sides(n: usize) -> Vec<(String, String)> {
        (0..n)
            .map(|i| (format!("front {i}"), "back".to_owned()))
            .collect()
    }

    /// Whether the Card Menu's Move to Deck… can be pressed, its menu
    /// opened in a window so its action's state reaches it
    /// (`testing::menu_item_sensitive`).
    fn move_item_sensitive(page: &ReviewPage) -> bool {
        let window = in_window(page);
        window.present();
        let sensitive =
            crate::testing::menu_item_sensitive(&page.imp().card_menu_button, "Move to Deck…");
        window.destroy();
        sensitive
    }

    #[test]
    fn move_to_deck_needs_another_deck() {
        crate::testing::with_widgets(|| {
            let (_tmp, _state, page, _) = page_of(2);
            assert!(!page.imp().can_move.get(), "the only deck");
            assert!(!move_item_sensitive(&page), "the menu item is insensitive");
            // Even if the page thought there was a deck (a stale flag): the
            // look at the decks finds none, says so and opens nothing.
            page.imp().can_move.set(true);
            page.move_current();
            assert!(!page.imp().asking.get());
            assert!(!page.imp().can_move.get(), "the flag is corrected");
            let (_tmp2, state2) = crate::testing::temporary_state();
            state2.create_deck("Other").unwrap();
            let (page2, _) = page_over(&state2, &sides(1));
            assert!(page2.imp().can_move.get(), "the deck has a neighbour");
            assert!(move_item_sensitive(&page2));
        });
    }

    #[test]
    fn a_moved_card_leaves_the_session_like_a_suspended_one_and_undo_brings_it_back() {
        crate::testing::with_widgets(|| {
            let (_tmp, state) = crate::testing::temporary_state();
            let other = state.create_deck("Other").unwrap();
            let (page, _) = page_over(&state, &sides(3));
            let _window = in_window(&page);
            let first = current(&page).unwrap();
            let from = state.card(&first).unwrap().deck_id;
            let left = |page: &ReviewPage| {
                page.imp()
                    .session
                    .borrow()
                    .as_ref()
                    .map_or(0, Run::remaining)
            };
            assert_eq!(left(&page), 3);

            page.finish_move(&first, &other);
            // In the other deck, no longer in the session, nothing answered.
            assert_eq!(state.card(&first).unwrap().deck_id, other.id);
            assert_ne!(current(&page).as_deref(), Some(first.as_str()));
            assert_eq!(left(&page), 2);
            assert_eq!(
                page.imp()
                    .session
                    .borrow()
                    .as_ref()
                    .unwrap()
                    .summary()
                    .cards,
                0
            );

            let on_screen = current(&page);
            // Undo: the card is back in its deck and comes next.
            let moved = vec![rappel_core::storage::Moved {
                card_id: first.clone(),
                from: from.clone(),
                to: other.id.clone(),
            }];
            undo_move(
                &state,
                &first,
                state.unmove_cards(&moved),
                &page.downgrade(),
                None,
            );
            assert_eq!(state.card(&first).unwrap().deck_id, from);
            assert_eq!(current(&page), on_screen, "it comes next, not at once");
            assert_eq!(left(&page), 3);

            // An Undo that could not move the card back (moved again since)
            // puts nothing back.
            page.finish_move(&first, &other);
            let again = state.create_deck("Third").unwrap();
            state
                .move_cards(std::slice::from_ref(&first), &again.id)
                .unwrap();
            let moved = vec![rappel_core::storage::Moved {
                card_id: first.clone(),
                from,
                to: other.id.clone(),
            }];
            undo_move(
                &state,
                &first,
                state.unmove_cards(&moved),
                &page.downgrade(),
                None,
            );
            assert_ne!(current(&page).as_deref(), Some(first.as_str()));
            assert_eq!(state.card(&first).unwrap().deck_id, again.id);
        });
    }

    #[test]
    fn a_card_deleted_elsewhere_leaves_the_session_instead_of_moving() {
        crate::testing::with_widgets(|| {
            let (_tmp, state) = crate::testing::temporary_state();
            let other = state.create_deck("Other").unwrap();
            let (page, _) = page_over(&state, &sides(2));
            let _window = in_window(&page);
            let first = current(&page).unwrap();
            state.delete_card(&first).unwrap();
            page.finish_move(&first, &other);
            assert_ne!(current(&page).as_deref(), Some(first.as_str()));
        });
    }

    #[test]
    fn an_undo_after_the_page_is_gone_restores_the_card_only_in_the_database() {
        crate::testing::with_widgets(|| {
            let (_tmp, state, page, ids) = page_of(1);
            let weak = page.downgrade();
            page.remove(&ids[0], Removal::Delete);
            drop(page);
            assert!(weak.upgrade().is_none());
            undo_removal(&state, &ids[0], Removal::Delete, &weak, None);
            assert!(state.card(&ids[0]).is_ok());
        });
    }

    #[test]
    fn an_undo_that_cannot_restore_the_card_leaves_it_deleted_and_the_session_alone() {
        crate::testing::with_widgets(|| {
            let (_tmp, state, page, ids) = page_of(1);
            let _window = in_window(&page);
            let deck = state.card(&ids[0]).unwrap().deck_id;
            page.remove(&ids[0], Removal::Delete);
            state.delete_deck(&deck).unwrap();
            // The deck is deleted: the card cannot come back into it.
            undo_removal(&state, &ids[0], Removal::Delete, &page.downgrade(), None);
            assert!(state.card(&ids[0]).is_err());
            assert_eq!(current(&page), None);
        });
    }

    #[test]
    fn a_removal_is_named_in_its_toast() {
        assert_eq!(Removal::Suspend.text(), "Card suspended");
        assert_eq!(Removal::Delete.text(), "Card deleted");
    }

    #[test]
    fn a_suspended_card_leaves_the_session_and_its_undo_brings_it_back() {
        crate::testing::with_widgets(|| {
            let (_tmp, state, page, _) = page_of(3);
            let _window = in_window(&page);
            let first = current(&page).unwrap();
            let summary_before = page.imp().session.borrow().as_ref().unwrap().summary();

            page.remove_current(Removal::Suspend);
            assert_ne!(current(&page).as_deref(), Some(first.as_str()));
            assert!(state.card(&first).unwrap().suspended);
            // Not counted as answered, and nothing was written to the log.
            let session = page.imp().session.borrow();
            assert_eq!(session.as_ref().unwrap().summary(), summary_before);
            // The card on screen stays queued until it is answered.
            assert_eq!(session.as_ref().unwrap().remaining(), 2);
            drop(session);
            assert_eq!(state.review_events(&first).unwrap().len(), 2);

            undo_removal(&state, &first, Removal::Suspend, &page.downgrade(), None);
            assert!(!state.card(&first).unwrap().suspended);
            // It is the card after the one on screen.
            let second = current(&page).unwrap();
            pass(&page);
            assert_eq!(current(&page).as_deref(), Some(first.as_str()));
            assert_ne!(second, first);
        });
    }

    #[test]
    fn deleting_the_last_card_shows_the_summary_and_undo_shows_the_card_again() {
        crate::testing::with_widgets(|| {
            let (_tmp, state, page, ids) = page_of(1);
            let _window = in_window(&page);
            page.remove(&ids[0], Removal::Delete);
            assert_eq!(current(&page), None);
            assert!(state.card(&ids[0]).is_err(), "the card is deleted");

            undo_removal(&state, &ids[0], Removal::Delete, &page.downgrade(), None);
            assert_eq!(current(&page).as_deref(), Some(ids[0].as_str()));
            assert!(state.card(&ids[0]).is_ok());
        });
    }

    #[test]
    fn a_removal_of_a_card_gone_elsewhere_just_moves_on() {
        crate::testing::with_widgets(|| {
            let (_tmp, state, page, ids) = page_of(2);
            let _window = in_window(&page);
            let first = current(&page).unwrap();
            state.delete_card(&first).unwrap();
            page.remove_current(Removal::Suspend);
            assert_ne!(current(&page).as_deref(), Some(first.as_str()));
            assert!(ids.contains(&current(&page).unwrap()));
        });
    }

    #[test]
    fn a_card_suspended_elsewhere_is_skipped_when_its_turn_comes() {
        crate::testing::with_widgets(|| {
            let (_tmp, state, page, ids) = page_of(2);
            let first = current(&page).unwrap();
            let other = ids.iter().find(|id| **id != first).unwrap().clone();
            state.set_suspended(&[other], true).unwrap();
            pass(&page);
            assert_eq!(current(&page), None, "the session is over");
        });
    }

    #[test]
    fn ctrl_z_takes_back_an_answer_without_showing_a_card_suspended_since() {
        crate::testing::with_widgets(|| {
            let (_tmp, state, page, _) = page_of(2);
            let first = current(&page).unwrap();
            pass(&page);
            let second = current(&page).unwrap();
            state
                .set_suspended(std::slice::from_ref(&first), true)
                .unwrap();

            page.undo();
            // The answer is taken back in the log, and the card the undo
            // would show is not studied.
            assert_eq!(undo_entries(&state, &first), 1);
            assert_eq!(current(&page).as_deref(), Some(second.as_str()));
        });
    }

    /// Reveals and passes the card on screen.
    fn pass(page: &ReviewPage) {
        page.reveal();
        page.grade(Grade::Pass);
    }

    fn current(page: &ReviewPage) -> Option<String> {
        page.imp().current.borrow().clone()
    }

    /// The text of the first label under `widget`, depth first.
    fn first_label(widget: &gtk::Widget) -> Option<String> {
        if let Some(label) = widget.downcast_ref::<gtk::Label>() {
            return Some(label.text().to_string());
        }
        let mut child = widget.first_child();
        while let Some(c) = child {
            if let Some(text) = first_label(&c) {
                return Some(text);
            }
            child = c.next_sibling();
        }
        None
    }

    /// The header's Undo button: the button bound to `review.undo`.
    fn undo_button(page: &ReviewPage) -> gtk::Button {
        let mut todo = vec![page.clone().upcast::<gtk::Widget>()];
        while let Some(widget) = todo.pop() {
            if let Some(button) = widget.downcast_ref::<gtk::Button>()
                && button.action_name().as_deref() == Some("review.undo")
            {
                return button.clone();
            }
            let mut child = widget.first_child();
            while let Some(c) = child {
                child = c.next_sibling();
                todo.push(c);
            }
        }
        panic!("the review page has an Undo button");
    }

    /// Whether the user can undo: the Undo button is sensitive, which its
    /// `GtkActionable` keeps equal to whether `review.undo` is enabled
    /// (#57). The session must agree. The page must be in a window, as in
    /// the app: seen with GTK 4.24, a button of a page without a root
    /// stays insensitive whatever the action says.
    fn can_undo(page: &ReviewPage) -> bool {
        assert!(page.root().is_some(), "put the page in a window first");
        let enabled = undo_button(page).is_sensitive();
        let in_session = page
            .imp()
            .session
            .borrow()
            .as_ref()
            .is_some_and(Run::can_undo);
        assert_eq!(enabled, in_session, "the Undo button follows the session");
        enabled
    }

    fn undo_entries(state: &AppState, card: &str) -> usize {
        let log = state.review_events(card).unwrap();
        log.iter()
            .filter(|e| matches!(e.kind, EventKind::Undo { .. }))
            .count()
    }

    #[test]
    fn undo_goes_back_to_the_first_answer() {
        crate::testing::with_widgets(|| {
            let (_tmp, state, page, ids) = page_of(3);
            let _window = in_window(&page);
            let before = state.card_state(&ids[0]).unwrap();
            assert!(!can_undo(&page));
            pass(&page);
            pass(&page);
            assert_eq!(current(&page).as_deref(), Some(ids[2].as_str()));
            assert!(can_undo(&page));
            page.undo();
            assert_eq!(current(&page).as_deref(), Some(ids[1].as_str()));
            page.undo();
            // The first card is back: front side, nothing revealed.
            assert_eq!(current(&page).as_deref(), Some(ids[0].as_str()));
            assert!(!page.imp().revealed.get());
            assert_eq!(page.imp().window_title.subtitle(), "3 left");
            assert!(!can_undo(&page));
            assert_eq!(state.card_state(&ids[0]).unwrap(), before);
        });
    }

    #[test]
    fn undo_from_the_summary_brings_the_last_card_back() {
        crate::testing::with_widgets(|| {
            let (_tmp, state, page, ids) = page_of(2);
            let _window = in_window(&page);
            pass(&page);
            pass(&page);
            let imp = page.imp();
            assert_eq!(imp.stack.visible_child_name().as_deref(), Some("summary"));
            assert!(current(&page).is_none());
            // The header's Undo button stays usable on the summary.
            assert!(can_undo(&page));
            page.undo();
            assert_eq!(imp.stack.visible_child_name().as_deref(), Some("card"));
            assert_eq!(current(&page).as_deref(), Some(ids[1].as_str()));
            let summary = imp.session.borrow().as_ref().unwrap().summary();
            assert_eq!((summary.cards, summary.answers), (1, 1));
            assert_eq!(undo_entries(&state, &ids[1]), 1);
        });
    }

    #[test]
    fn undo_shows_the_card_as_edited_meanwhile() {
        crate::testing::with_widgets(|| {
            let (_tmp, state, page, ids) = page_of(2);
            let _window = in_window(&page);
            pass(&page);
            state
                .edit_card_with_tags(&ids[0], "edited front", "back", None)
                .unwrap();
            assert!(can_undo(&page));
            page.undo();
            let front = first_label(page.imp().front_box.upcast_ref());
            assert_eq!(front.as_deref(), Some("edited front"));
        });
    }

    #[test]
    fn rapid_repeated_undo_stops_at_the_first_answer() {
        crate::testing::with_widgets(|| {
            let (_tmp, state, page, ids) = page_of(3);
            let _window = in_window(&page);
            for _ in 0..3 {
                pass(&page);
            }
            for _ in 0..5 {
                page.undo();
            }
            assert!(!can_undo(&page));
            assert_eq!(current(&page).as_deref(), Some(ids[0].as_str()));
            let undone: usize = ids.iter().map(|id| undo_entries(&state, id)).sum();
            assert_eq!(undone, 3, "one undo per answer, no more");
        });
    }

    #[test]
    fn an_answer_that_is_no_longer_the_latest_is_not_undone() {
        crate::testing::with_widgets(|| {
            let (_tmp, state, page, ids) = page_of(2);
            let _window = in_window(&page);
            pass(&page);
            // Something else (an import, say) made a newer entry.
            state
                .reset_card(&ids[0], rappel_core::time::now_ms())
                .unwrap();
            let log = state.review_events(&ids[0]).unwrap().len();
            page.undo();
            assert_eq!(state.review_events(&ids[0]).unwrap().len(), log);
            assert_eq!(current(&page).as_deref(), Some(ids[1].as_str()));
            assert!(!can_undo(&page));
        });
    }

    #[test]
    fn the_summary_shows_cards_rate_and_time() {
        let summary = Summary {
            cards: 1_234,
            answers: 18,
            passes: 15,
            time: Duration::from_secs(250),
        };
        let values = |tiles: [Tile; 3]| tiles.map(|t| (t.value, t.caption, t.accessible));
        assert_eq!(
            values(summary_tiles(&summary, false).unwrap()),
            [
                (
                    "1,234".to_owned(),
                    "Cards reviewed".to_owned(),
                    "Cards reviewed: 1,234".to_owned()
                ),
                (
                    "83%".to_owned(),
                    "Pass rate".to_owned(),
                    "Pass rate: 83%".to_owned()
                ),
                (
                    "4 min 10 s".to_owned(),
                    "Time".to_owned(),
                    "Time: 4 min 10 s".to_owned()
                ),
            ]
        );
        let one = Summary {
            cards: 1,
            answers: 1,
            passes: 0,
            time: Duration::from_secs(3),
        };
        let [cards, passed, time] = summary_tiles(&one, false).unwrap();
        assert_eq!(
            (cards.value.as_str(), cards.caption.as_str()),
            ("1", "Card reviewed")
        );
        assert_eq!(passed.value, "0%");
        assert_eq!(time.value, "3 s");
        // Every card suspended or deleted: nothing to sum up.
        assert_eq!(summary_tiles(&Summary::default(), false), None);
    }

    #[test]
    fn the_summary_icon_matches_the_blueprint() {
        crate::testing::with_widgets(|| {
            let page: ReviewPage = glib::Object::new();
            assert_eq!(
                page.imp().summary_page.icon_name().as_deref(),
                Some(SUMMARY_ICON)
            );
        });
    }

    #[test]
    fn the_subtitle_counts_what_is_left_with_grouped_digits() {
        assert_eq!(left_text(0), "0 left");
        assert_eq!(left_text(3), "3 left");
        assert_eq!(left_text(1_234), "1,234 left");
    }

    #[test]
    fn the_answer_fades_in_and_slides_up_into_place() {
        assert_eq!(reveal_step(0.0), (0.0, 12));
        assert_eq!(reveal_step(0.5), (0.5, 6));
        assert_eq!(reveal_step(1.0), (1.0, 0));
        // Out of range (or not a number) ends shown, in place.
        assert_eq!(reveal_step(-1.0), (0.0, 12));
        assert_eq!(reveal_step(2.0), (1.0, 0));
        assert_eq!(reveal_step(f64::NAN), (1.0, 0));
        // It only ever comes closer to its place.
        let steps: Vec<_> = (0..=100)
            .map(|i| reveal_step(f64::from(i) / 100.0))
            .collect();
        assert!(
            steps
                .windows(2)
                .all(|w| w[0].0 <= w[1].0 && w[0].1 >= w[1].1)
        );
    }

    /// The buttons' tooltips name the keys the page binds to the same act
    /// (#105), and no hint is left on the page.
    #[test]
    fn the_buttons_say_their_keys_in_their_tooltips() {
        crate::testing::with_widgets(|| {
            let page: ReviewPage = glib::Object::new();
            let imp = page.imp();
            for (button, tooltip, trigger, act) in [
                (
                    &*imp.reveal_button,
                    "Show Answer (Space)",
                    "space",
                    Act::Reveal,
                ),
                (&*imp.fail_button, "Fail (1)", "1", Act::Grade(Grade::Fail)),
                (&*imp.pass_button, "Pass (2)", "2", Act::Grade(Grade::Pass)),
                (
                    &*imp.undo_button,
                    "Undo Last Answer (Ctrl+Z)",
                    "<Control>z",
                    Act::Undo,
                ),
                (&*imp.edit_button, "Edit Card (E)", "e", Act::Edit),
            ] {
                assert_eq!(button.tooltip_text().as_deref(), Some(tooltip));
                let (_, bound) = KEYS.iter().find(|(t, _)| *t == trigger).unwrap();
                let revealed = act != Act::Reveal;
                assert_eq!(key_action(*bound, revealed, true), Some(act), "{trigger}");
            }
        });
        // Space also passes, as the Pass button's description says.
        assert_eq!(
            key_action(Key::Space, true, true),
            Some(Act::Grade(Grade::Pass))
        );
        // Nothing under or beside the buttons any more.
        let blueprint = include_str!("../../data/review_page.blp");
        assert!(!blueprint.contains("_hint"));
    }

    /// GTK has no public getter for an accessible description, so the
    /// buttons' descriptions are checked in the template's source.
    #[test]
    fn the_buttons_describe_their_keys() {
        let blueprint = include_str!("../../data/review_page.blp");
        for description in ["Press Space", "Press 1", "Press 2 or Space"] {
            assert!(
                blueprint.contains(&format!("description: _(\"{description}\");")),
                "{description}"
            );
        }
    }

    /// No swipe grades or reveals (SPEC §4.8, ADR 0058): nothing on the
    /// review page, front or back, images and audio controls included,
    /// has a swipe, drag, pan or long-press gesture of Rappel's. GTK's own
    /// are allowed, as they only scroll (a scrolled window's kinetic
    /// scrolling, a scrollbar), move the window (the header's window
    /// handle) or open Back's history menu. `AdwNavigationView`'s swipe back
    /// is on the view around the page and only leaves it, as Back does.
    #[test]
    fn no_gesture_on_the_review_page_can_grade() {
        /// GTK's own widgets that come with such gestures: they only
        /// scroll, move the window or open Back's history.
        const GTK_OWN: [&str; 4] = [
            "GtkScrolledWindow",
            "GtkRange",
            "GtkWindowHandle",
            "AdwBackButton",
        ];
        fn is_gesture(controller: &glib::Object) -> bool {
            controller.is::<gtk::GestureSwipe>()
                || controller.is::<gtk::GestureDrag>()
                || controller.is::<gtk::GestureLongPress>()
        }
        fn count(widget: &gtk::Widget) -> usize {
            let controllers = widget.observe_controllers();
            (0..controllers.n_items())
                .filter_map(|i| controllers.item(i))
                .filter(is_gesture)
                .count()
        }
        fn gestures(widget: &gtk::Widget, found: &mut Vec<String>) {
            let kind = widget.type_();
            // A widget of GTK's own may have the gestures a new one of its
            // type has, and no more: one Rappel added would be one more.
            let allowed = if GTK_OWN.contains(&kind.name()) {
                glib::Object::with_type(kind)
                    .downcast::<gtk::Widget>()
                    .map_or(0, |fresh| count(&fresh))
            } else {
                0
            };
            let has = count(widget);
            if has > allowed {
                found.push(format!(
                    "{has} gestures on {}, {allowed} allowed",
                    kind.name()
                ));
            }
            let mut child = widget.first_child();
            while let Some(c) = child {
                gestures(&c, found);
                child = c.next_sibling();
            }
        }
        crate::testing::with_widgets(|| {
            let image = format!("![a picture](media:{}.png)", "a".repeat(64));
            let audio = format!("[a sound](media:{}.wav)", "b".repeat(64));
            let (_tmp, _state, page, _ids) =
                page_with(&[(format!("{image}\n\n{audio}"), format!("back\n\n{image}"))]);
            page.reveal();
            let mut found = Vec::new();
            gestures(page.upcast_ref(), &mut found);
            assert!(found.is_empty(), "{found:?}");
        });
    }

    #[test]
    fn the_answer_is_scrolled_into_view_only_when_it_starts_too_low() {
        // Plenty of the answer in view: no scroll.
        assert!(answer_scroll(200.0, 500.0).abs() < f64::EPSILON);
        assert!(answer_scroll(404.0, 500.0).abs() < f64::EPSILON);
        // Under the view's end, or too little of it shown: to its start.
        assert!((answer_scroll(450.0, 500.0) - 438.0).abs() < f64::EPSILON);
        assert!((answer_scroll(900.0, 240.0) - 888.0).abs() < f64::EPSILON);
        // A short view needs only half of itself shown.
        assert!(answer_scroll(110.0, 240.0).abs() < f64::EPSILON);
        // Not laid out: nothing.
        assert!(answer_scroll(900.0, 0.0).abs() < f64::EPSILON);
    }

    #[test]
    fn a_short_window_makes_the_summary_compact_without_its_icon() {
        crate::testing::with_widgets(|| {
            let page: ReviewPage = glib::Object::new();
            let imp = page.imp();
            assert!(!imp.summary_page.has_css_class("compact"));
            imp.short_breakpoint.emit_by_name::<()>("apply", &[]);
            assert!(imp.summary_page.has_css_class("compact"));
            assert_eq!(imp.summary_page.icon_name(), None);
            imp.short_breakpoint.emit_by_name::<()>("unapply", &[]);
            assert!(!imp.summary_page.has_css_class("compact"));
            assert_eq!(imp.summary_page.icon_name().as_deref(), Some(SUMMARY_ICON));
        });
    }

    /// `page` in a presented window, with animations on, once its card is
    /// mapped (an animation on an unmapped widget is skipped).
    fn on_screen(page: &ReviewPage) -> adw::Window {
        gtk::Settings::default()
            .expect("GTK is started")
            .set_gtk_enable_animations(true);
        let window = adw::Window::builder()
            .default_width(1000)
            .default_height(720)
            .content(page)
            .build();
        window.present();
        assert!(
            crate::testing::settle_until(
                || page.imp().card_area.is_mapped(),
                Duration::from_secs(5)
            ),
            "the page did not show"
        );
        window
    }

    fn animation(page: &ReviewPage) -> adw::AnimationState {
        page.imp()
            .reveal_animation
            .get()
            .expect("made in constructed")
            .state()
    }

    /// The reveal really plays on screen, and the next card, or an undo,
    /// ends it with the back hidden. No main-loop iteration runs between
    /// the calls and the checks, so the state cannot move on by itself.
    #[test]
    fn the_reveal_plays_on_screen_and_the_next_card_or_an_undo_ends_it() {
        crate::testing::with_widgets(|| {
            let (_tmp, _state, page, _) = page_of(3);
            let window = on_screen(&page);
            let imp = page.imp();

            page.reveal();
            assert_eq!(animation(&page), adw::AnimationState::Playing);
            assert!(imp.back_part.is_visible());
            assert!(imp.back_part.opacity() < 1.0);
            page.grade(Grade::Pass);
            assert_eq!(animation(&page), adw::AnimationState::Finished);
            assert!(!imp.back_part.is_visible());

            page.reveal();
            assert_eq!(animation(&page), adw::AnimationState::Playing);
            page.undo();
            assert_eq!(animation(&page), adw::AnimationState::Finished);
            assert!(!imp.back_part.is_visible());
            assert!(!imp.revealed.get());

            // Left alone, it finishes with the back fully shown in place.
            page.reveal();
            assert!(crate::testing::settle_until(
                || animation(&page) == adw::AnimationState::Finished,
                Duration::from_secs(5)
            ));
            assert!((imp.back_part.opacity() - 1.0).abs() < f64::EPSILON);
            assert_eq!(imp.back_part.margin_top(), 0);
            window.destroy();
        });
    }

    /// The reveal animation on the largest backs the page lays out: a 32 KiB
    /// paragraph, 200 inline formulas, a list of 2,000 items. Its frames
    /// change the back's opacity and top margin; the margin re-measures only
    /// the back's box and its ancestors (GTK keeps the labels' and formulas'
    /// measures), so the frames cost what drawing costs (ADR 0045). Timed
    /// in CPU time once the back's first layout is done; about 0 ms, 180 ms
    /// and 10 ms here (debug build, Xvfb), the same as an animation of the
    /// opacity alone. The bound leaves room for slow machines.
    #[test]
    fn a_reveal_of_the_largest_backs_stays_cheap() {
        use std::fmt::Write as _;
        let sides = [
            ("32 KiB paragraph", "word ".repeat(MAX_SHOWN_BYTES / 5)),
            (
                "200 formulas",
                (0..200).fold(String::new(), |mut side, i| {
                    let _ = write!(side, "$x_{{{i}}}^2$ and ");
                    side
                }),
            ),
            (
                "2,000 items",
                (0..2_000).fold(String::new(), |mut side, i| {
                    let _ = writeln!(side, "- item {i}");
                    side
                }),
            ),
        ];
        let times = crate::testing::with_widgets(move || {
            sides.map(|(what, back)| {
                assert!(back.len() <= MAX_SHOWN_BYTES, "{what}");
                let (_tmp, _state, page, _) = page_with(&[("front".to_owned(), back)]);
                let window = on_screen(&page);
                let finished = || animation(&page) == adw::AnimationState::Finished;
                page.reveal();
                assert!(crate::testing::settle_until(
                    finished,
                    Duration::from_secs(20)
                ));
                // Checked on this thread, which did the work (ADR 0049),
                // and measured again if a burst of load slowed one run
                // (ADR 0063).
                let mut used = Duration::ZERO;
                let verdict = crate::testing::cpu_within_retried(Duration::from_secs(2), || {
                    let (time, settled) = crate::testing::cpu_time_of(|| {
                        page.animate_reveal();
                        crate::testing::settle_until(finished, Duration::from_secs(20))
                    });
                    assert!(settled);
                    used = time;
                    time
                })
                .map(|_| ());
                window.destroy();
                (what, used, verdict)
            })
        });
        for (what, used, verdict) in times {
            crate::testing::print_timing(&format!("reveal animation, {what}"), used);
            if let Err(message) = verdict {
                panic!("reveal animation, {what}: {message}");
            }
        }
    }

    #[test]
    fn the_back_is_shown_whole_at_the_reveal_and_hidden_for_the_next_card() {
        crate::testing::with_widgets(|| {
            let (_tmp, _state, page, _) = page_of(2);
            let imp = page.imp();
            assert!(!imp.back_part.is_visible());
            assert_eq!(imp.buttons.visible_child_name().as_deref(), Some("reveal"));
            page.reveal();
            // Not on screen, so libadwaita skips the animation to its end.
            assert!(imp.back_part.is_visible());
            assert!((imp.back_part.opacity() - 1.0).abs() < f64::EPSILON);
            assert_eq!(imp.back_part.margin_top(), 0);
            assert_eq!(imp.buttons.visible_child_name().as_deref(), Some("grade"));
            assert_eq!(
                first_label(imp.back_box.upcast_ref()).as_deref(),
                Some("back")
            );
            page.grade(Grade::Pass);
            assert!(!imp.back_part.is_visible());
            assert_eq!(imp.buttons.visible_child_name().as_deref(), Some("reveal"));
            assert_eq!(imp.window_title.subtitle(), "1 left");
        });
    }

    #[test]
    fn the_summary_replaces_the_bar_and_shows_the_figures() {
        crate::testing::with_widgets(|| {
            let (_tmp, _state, page, _) = page_of(2);
            let imp = page.imp();
            assert!(imp.bottom_bar.is_visible());
            pass(&page);
            pass(&page);
            assert_eq!(imp.stack.visible_child_name().as_deref(), Some("summary"));
            assert!(!imp.bottom_bar.is_visible());
            assert!(imp.summary_figures.is_visible());
            assert_eq!(imp.cards_value.label(), "2");
            assert_eq!(imp.cards_caption.label(), "Cards reviewed");
            assert_eq!(imp.passed_value.label(), "100%");
            assert!(
                imp.summary_page
                    .description()
                    .unwrap_or_default()
                    .is_empty()
            );
            assert_eq!(imp.window_title.subtitle(), "");
            // Undo brings the card and the bar back.
            page.undo();
            assert!(imp.bottom_bar.is_visible());
            assert_eq!(imp.buttons.visible_child_name().as_deref(), Some("reveal"));
        });
    }

    #[test]
    fn a_session_with_nothing_answered_says_so() {
        crate::testing::with_widgets(|| {
            let (_tmp, _state, page, ids) = page_of(1);
            let _window = in_window(&page);
            page.remove(&ids[0], Removal::Suspend);
            let imp = page.imp();
            assert_eq!(imp.stack.visible_child_name().as_deref(), Some("summary"));
            assert!(!imp.summary_figures.is_visible());
            assert_eq!(
                imp.summary_page.description().as_deref(),
                Some(nothing_reviewed().as_str())
            );
        });
    }

    /// Adds a card to `deck` with `tags`, learned long ago so it is due.
    fn due_card(state: &AppState, deck: &str, front: &str, tags: &[&str]) -> String {
        let tags: Vec<String> = tags.iter().map(|&t| t.to_owned()).collect();
        let id = state
            .add_card_with_tags(deck, front, "back", &tags)
            .unwrap()
            .card
            .id;
        let now = rappel_core::time::now_ms();
        let day = crate::ui::time::DAY_MS;
        for at in [now - 60 * day, now - 60 * day + 11 * 60 * 1000] {
            state
                .record_review(&id, Grade::Pass, Duration::ZERO, at)
                .unwrap();
        }
        id
    }

    fn tag_named(state: &AppState, name: &str) -> Tag {
        state
            .tags()
            .unwrap()
            .into_iter()
            .find(|t| t.name == name)
            .unwrap()
    }

    /// A tag's session shows the tag in its title and studies its due
    /// cards of every deck that is not archived, writing each answer to the
    /// log like a deck's; it is no deck's session (ADR 0069).
    #[test]
    fn a_tag_session_studies_the_tagged_cards_of_every_active_deck() {
        crate::testing::with_widgets(|| {
            let (_tmp, state) = crate::testing::temporary_state();
            let a = state.create_deck("A").unwrap();
            let b = state.create_deck("B").unwrap();
            let gone = state.create_deck("Gone").unwrap();
            let a1 = due_card(&state, &a.id, "a1", &["verbs"]);
            due_card(&state, &a.id, "a2", &["nouns"]);
            let b1 = due_card(&state, &b.id, "b1", &["verbs", "x"]);
            due_card(&state, &gone.id, "g1", &["verbs"]);
            state.archive_deck(&gone.id).unwrap();
            let tag = tag_named(&state, "verbs");
            let page =
                ReviewPage::for_tag(state.clone(), tag, StartedFrom::AllCards, |_| {}).unwrap();
            let imp = page.imp();
            assert_eq!(imp.window_title.title(), "Tagged “\u{2068}verbs\u{2069}”");
            assert_eq!(page.deck_id(), None);
            assert_eq!(
                imp.done_button.label().as_deref(),
                Some("_Back to All Cards")
            );
            let mut shown = Vec::new();
            while let Some(id) = current(&page) {
                assert!(shown.len() < 10, "the session does not end");
                shown.push(id.clone());
                pass(&page);
                // Written as in a deck's session.
                assert_eq!(state.review_events(&id).unwrap().len(), 3);
            }
            shown.sort();
            let mut want = vec![a1, b1];
            want.sort();
            assert_eq!(shown, want);
            assert_eq!(imp.summary_page.title(), "Session Complete");
            assert!(imp.summary_figures.is_visible());
        });
    }

    /// With nothing to study, a session opens on the "all done" page,
    /// worded for its tag or deck, with the button back to where it began.
    #[test]
    fn a_session_with_nothing_to_study_says_so_for_its_tag_or_deck() {
        crate::testing::with_widgets(|| {
            let (_tmp, state) = crate::testing::temporary_state();
            let deck = state.create_deck("Spanish").unwrap();
            let gone = state.create_deck("Gone").unwrap();
            due_card(&state, &gone.id, "g1", &["verbs"]);
            state.archive_deck(&gone.id).unwrap();
            let tag = tag_named(&state, "verbs");
            let page =
                ReviewPage::for_tag(state.clone(), tag, StartedFrom::DeckPage, |_| {}).unwrap();
            let _window = in_window(&page);
            let imp = page.imp();
            assert_eq!(imp.stack.visible_child_name().as_deref(), Some("summary"));
            assert!(!imp.summary_figures.is_visible());
            assert_eq!(imp.summary_page.title(), "Nothing to Study");
            assert_eq!(
                imp.summary_page.description().as_deref(),
                Some("No cards tagged “\u{2068}verbs\u{2069}” are left to study now.")
            );
            assert_eq!(imp.done_button.label().as_deref(), Some("_Back to Deck"));

            let page = ReviewPage::new(state.clone(), deck, |_| {}).unwrap();
            let _window = in_window(&page);
            let imp = page.imp();
            assert_eq!(imp.summary_page.title(), "Nothing to Study");
            assert_eq!(
                imp.summary_page.description().as_deref(),
                Some("No cards of “\u{2068}Spanish\u{2069}” are left to study now.")
            );
            assert_eq!(imp.done_button.label().as_deref(), Some("_Back to Deck"));
        });
    }

    /// A tag's name in the "all done" page's description, which is Pango
    /// markup, is drawn as it is: escaped, never parsed.
    #[test]
    fn a_tag_name_in_the_all_done_page_is_not_markup() {
        crate::testing::with_widgets(|| {
            let (_tmp, state) = crate::testing::temporary_state();
            let deck = state.create_deck("D").unwrap();
            let card = state
                .add_card_with_tags(&deck.id, "q", "a", &["a&b <i>".to_owned()])
                .unwrap();
            state.set_suspended(&[card.card.id], true).unwrap();
            let tag = tag_named(&state, "a&b <i>");
            let page =
                ReviewPage::for_tag(state.clone(), tag, StartedFrom::AllCards, |_| {}).unwrap();
            let mut texts = Vec::new();
            texts.extend(crate::testing::label_texts(
                page.imp().summary_page.upcast_ref(),
            ));
            assert!(
                texts.contains(
                    &"No cards tagged “\u{2068}a&b <i>\u{2069}” are left to study now.".to_owned()
                ),
                "{texts:?}"
            );
        });
    }

    /// Archiving a deck during a tag's session drops that deck's cards,
    /// the one on screen too, which leaves as a suspended one does; the
    /// other decks' cards stay (ADR 0068, 0069).
    #[test]
    fn a_deck_archived_during_a_tag_session_leaves_it() {
        crate::testing::with_widgets(|| {
            let (_tmp, state) = crate::testing::temporary_state();
            let a = state.create_deck("A").unwrap();
            let b = state.create_deck("B").unwrap();
            let a_cards = [
                due_card(&state, &a.id, "a1", &["t"]),
                due_card(&state, &a.id, "a2", &["t"]),
            ];
            let b1 = due_card(&state, &b.id, "b1", &["t"]);
            let page = ReviewPage::for_tag(
                state.clone(),
                tag_named(&state, "t"),
                StartedFrom::AllCards,
                |_| {},
            )
            .unwrap();
            let on_screen = current(&page).unwrap();
            let deck = state.card(&on_screen).unwrap().deck_id;
            state.archive_deck(&deck).unwrap();
            page.leave_deck(&deck);
            let mut shown = Vec::new();
            while let Some(id) = current(&page) {
                assert!(shown.len() < 10, "the session does not end");
                shown.push(id);
                pass(&page);
            }
            let want: Vec<String> = if deck == a.id {
                vec![b1]
            } else {
                a_cards.to_vec()
            };
            let mut shown_sorted = shown.clone();
            shown_sorted.sort();
            let mut want_sorted = want;
            want_sorted.sort();
            assert_eq!(shown_sorted, want_sorted);
            // Nothing of the archived deck was answered.
            assert_eq!(state.review_events(&on_screen).unwrap().len(), 2);
            // A deck's own session is not changed by it.
            let c = state.create_deck("C").unwrap();
            due_card(&state, &c.id, "c1", &[]);
            let deck_page = ReviewPage::new(state.clone(), c.clone(), |_| {}).unwrap();
            let before = current(&deck_page);
            assert!(before.is_some());
            deck_page.leave_deck(&c.id);
            assert_eq!(current(&deck_page), before);
        });
    }

    /// After its deck is archived, a card does not come back into a tag's
    /// session by an Undo of its answer (the log is undone, the card is not
    /// shown) nor by the Undo of a Suspend, a Delete or a Move
    /// (`put_back`), ADR 0069.
    #[test]
    fn an_undo_never_brings_back_a_card_of_a_deck_archived_since() {
        crate::testing::with_widgets(|| {
            let (_tmp, state) = crate::testing::temporary_state();
            let mut by_deck = std::collections::HashMap::new();
            for name in ["A", "B"] {
                let deck = state.create_deck(name).unwrap();
                let cards: Vec<String> = (0..2)
                    .map(|i| due_card(&state, &deck.id, &format!("{name}{i}"), &["t"]))
                    .collect();
                by_deck.insert(deck.id, cards);
            }
            let page = ReviewPage::for_tag(
                state.clone(),
                tag_named(&state, "t"),
                StartedFrom::AllCards,
                |_| {},
            )
            .unwrap();
            let first = current(&page).unwrap();
            let deck = state.card(&first).unwrap().deck_id;
            pass(&page);
            state.archive_deck(&deck).unwrap();
            page.leave_deck(&deck);
            let archived = &by_deck[&deck];
            // The Undo of the answer: written to the log, not shown again.
            page.undo();
            assert_eq!(undo_entries(&state, &first), 1);
            assert!(current(&page).is_none_or(|c| !archived.contains(&c)));
            // The Undo of a removal puts the card back: not this one.
            let other = archived.iter().find(|c| **c != first).unwrap();
            page.put_back(other);
            let mut shown = Vec::new();
            while let Some(id) = current(&page) {
                assert!(shown.len() < 10, "the session does not end");
                assert!(!archived.contains(&id), "{id} of the archived deck");
                shown.push(id);
                pass(&page);
            }
            assert_eq!(shown.len(), 2, "the other deck's cards");
        });
    }

    #[test]
    fn the_session_complete_title_matches_the_blueprint() {
        assert!(
            include_str!("../../data/review_page.blp")
                .contains(&format!("title: _(\"{}\");", session_complete()))
        );
    }

    /// The action each key's menu item runs, for the hints' test.
    fn menu_action(key: Key) -> Option<&'static str> {
        match key {
            Key::Edit => Some("review.edit"),
            Key::Info => Some("review.card-info"),
            Key::Suspend => Some("review.suspend"),
            Key::Delete => Some("review.delete"),
            _ => None,
        }
    }

    /// (action, accel) of every item of `menu` and its sections.
    fn menu_items(menu: &gtk::gio::MenuModel, out: &mut Vec<(String, Option<String>)>) {
        let string = |i, name| {
            menu.item_attribute_value(i, name, Some(glib::VariantTy::STRING))
                .and_then(|v| v.get::<String>())
        };
        for i in 0..menu.n_items() {
            if let Some(section) = menu.item_link(i, "section") {
                menu_items(&section, out);
            } else if let Some(action) = string(i, "action") {
                out.push((action, string(i, "accel")));
            }
        }
    }

    #[test]
    fn the_card_menu_shows_the_keys_the_page_binds() {
        crate::testing::with_widgets(|| {
            let page: ReviewPage = glib::Object::new();
            let menu = page
                .imp()
                .card_menu_button
                .menu_model()
                .expect("the card menu button has its menu");
            let mut items = Vec::new();
            menu_items(&menu, &mut items);
            assert_eq!(items.len(), 5);
            for (action, accel) in items {
                // Move to Deck… has no key.
                if action == "review.move" {
                    assert_eq!(accel, None);
                    continue;
                }
                let accel = accel.unwrap_or_else(|| panic!("{action} shows no key"));
                let bound = KEYS
                    .iter()
                    .find(|(trigger, _)| *trigger == accel)
                    .unwrap_or_else(|| panic!("{accel} is not one of the page's keys"));
                assert_eq!(menu_action(bound.1), Some(action.as_str()));
                assert!(gtk::accelerator_parse(&accel).is_some());
            }
            // And GTK's menu shows them: each item's GtkModelButton has the
            // key as its `accel` (what it displays). This relies on GTK
            // internals: GtkModelButton is a private type (no public header
            // or Rust binding), found by its type name, and `accel` is its
            // private property. GTK offers no public way to read what a
            // menu item shows, so a GTK update may change this part (the
            // check of the menu model above is public API and stays).
            let popover = gtk::PopoverMenu::from_model(Some(&menu));
            let mut shown = Vec::new();
            let mut todo = vec![popover.upcast::<gtk::Widget>()];
            while let Some(widget) = todo.pop() {
                if widget.type_().name() == "GtkModelButton" {
                    shown.push(widget.property::<Option<String>>("accel"));
                }
                let mut child = widget.first_child();
                while let Some(c) = child {
                    child = c.next_sibling();
                    todo.push(c);
                }
            }
            shown.sort();
            assert_eq!(
                shown,
                [
                    None,
                    Some("Delete".to_owned()),
                    Some("e".to_owned()),
                    Some("i".to_owned()),
                    Some("s".to_owned())
                ]
            );
        });
    }

    /// The files a write would change (the database and its write-ahead
    /// log, with their modification times; not the `-shm` index, where
    /// readers keep their read marks) and another connection's
    /// `data_version`, which changes when anything commits.
    fn on_disk(state: &AppState, observer: &rusqlite::Connection) -> (Vec<Option<Vec<u8>>>, i64) {
        let db = state.data_dir().join("rappel.sqlite3");
        let wal = state.data_dir().join("rappel.sqlite3-wal");
        let files = [db, wal]
            .iter()
            .flat_map(|path| {
                let bytes = std::fs::read(path).ok();
                let modified = std::fs::metadata(path)
                    .and_then(|m| m.modified())
                    .ok()
                    .map(|t| format!("{t:?}").into_bytes());
                [bytes, modified]
            })
            .collect();
        let version = observer
            .query_row("PRAGMA data_version", [], |r| r.get(0))
            .unwrap();
        (files, version)
    }

    fn practice_of(state: &AppState, deck: &Deck) -> ReviewPage {
        let studied = Studied::Deck {
            deck: deck.clone(),
            from: StartedFrom::DeckPage,
        };
        ReviewPage::practice(state.clone(), studied, |_| {}).unwrap()
    }

    /// A practice through the page, with fails, passes and undos (Ctrl+Z
    /// and the summary's Undo), writes nothing: the database files and
    /// every card's log are as they were, and no connection committed
    /// (ADR 0070). Its banner and summary say so.
    #[test]
    fn a_practice_on_the_page_writes_nothing_and_says_so() {
        crate::testing::with_widgets(|| {
            let (_tmp, state, page, ids) = page_of(3);
            drop(page);
            let deck = state.decks().unwrap().remove(0);
            let observer =
                rusqlite::Connection::open(state.data_dir().join("rappel.sqlite3")).unwrap();
            let logs: Vec<usize> = ids
                .iter()
                .map(|id| state.review_events(id).unwrap().len())
                .collect();
            let before = on_disk(&state, &observer);
            let changes = state.total_changes();

            let page = practice_of(&state, &deck);
            let _window = in_window(&page);
            let imp = page.imp();
            assert!(page.is_practice());
            assert!(imp.banner.is_revealed());
            assert_eq!(imp.banner.title(), "Practice — nothing is rescheduled");
            assert_eq!(page.title(), "Practice");
            assert_eq!(imp.window_title.title(), "Deck");
            let mut steps = 0;
            while current(&page).is_some() {
                assert!(steps < 50, "the practice does not end");
                page.reveal();
                match steps % 4 {
                    0 => page.grade(Grade::Fail),
                    2 => {
                        page.grade(Grade::Pass);
                        assert_eq!(page.key(Key::Undo), glib::Propagation::Stop);
                    }
                    _ => page.grade(Grade::Pass),
                }
                steps += 1;
            }
            assert_eq!(imp.stack.visible_child_name().as_deref(), Some("summary"));
            assert_eq!(imp.summary_page.title(), "Practice Complete");
            assert_eq!(
                imp.summary_page.description().as_deref(),
                Some("Practice: nothing was rescheduled")
            );
            assert!(imp.summary_figures.get_visible());
            assert_eq!(imp.cards_caption.label(), "Cards practised");
            assert_eq!(imp.cards_value.label(), "3");
            // The cards and the pass rate; the words replace the banner.
            assert!(!imp.time_tile.get_visible());
            assert!(!imp.banner.is_revealed());
            // No Practice again from a practice.
            assert!(!imp.practice_button.get_visible());
            // Undo from the summary brings the last card back, in memory.
            assert!(can_undo(&page));
            page.undo();
            assert!(current(&page).is_some());
            assert!(imp.banner.is_revealed());

            assert_eq!(on_disk(&state, &observer), before);
            assert_eq!(state.total_changes(), changes);
            let after: Vec<usize> = ids
                .iter()
                .map(|id| state.review_events(id).unwrap().len())
                .collect();
            assert_eq!(after, logs);
        });
    }

    /// A practice shows every card, due or not: here none is due, and the
    /// least well remembered comes first.
    #[test]
    fn a_practice_shows_cards_that_are_not_due_least_remembered_first() {
        crate::testing::with_widgets(|| {
            let (_tmp, state) = crate::testing::temporary_state();
            let deck = state.create_deck("Deck").unwrap();
            let now = rappel_core::time::now_ms();
            let mut ids = Vec::new();
            for (front, hours_ago) in [("fresh", 2), ("older", 6)] {
                let id = state
                    .add_card_with_tags(&deck.id, front, "b", &[])
                    .unwrap()
                    .card
                    .id;
                let at = now - hours_ago * 3_600_000;
                for t in [at, at + 11 * 60 * 1000] {
                    state
                        .record_review(&id, Grade::Pass, Duration::ZERO, t)
                        .unwrap();
                }
                ids.push(id);
            }
            assert!(
                state
                    .study_input(&StudyScope::deck(&deck.id), now)
                    .unwrap()
                    .is_empty()
            );
            let page = practice_of(&state, &deck);
            assert_eq!(current(&page).as_ref(), Some(&ids[1]));
            pass(&page);
            assert_eq!(current(&page).as_ref(), Some(&ids[0]));
        });
    }

    /// A Fail brings the card back a few cards later, as the core's
    /// practice orders it, and Undo steps back to it.
    #[test]
    fn a_failed_card_comes_back_later_in_the_practice() {
        crate::testing::with_widgets(|| {
            let (_tmp, state, page, _) = page_of(5);
            drop(page);
            let deck = state.decks().unwrap().remove(0);
            let page = practice_of(&state, &deck);
            let first = current(&page).unwrap();
            page.reveal();
            page.grade(Grade::Fail);
            let mut shown = Vec::new();
            for _ in 0..rappel_core::scheduling::RELEARN_GAP {
                shown.push(current(&page).unwrap());
                pass(&page);
            }
            assert!(!shown.contains(&first));
            assert_eq!(current(&page), Some(first));
            page.undo();
            assert_eq!(current(&page).as_ref(), shown.last());
        });
    }

    /// The "all done" page offers Practice (and so does "Session
    /// Complete"), which takes the page's place: Back from the practice goes
    /// where Back went. A scope with nothing to practise does not offer it.
    #[test]
    fn the_all_done_page_offers_practice_in_its_place() {
        crate::testing::with_widgets(|| {
            let (_tmp, state) = crate::testing::temporary_state();
            let deck = state.create_deck("Spanish").unwrap();
            let card = state.add_card_with_tags(&deck.id, "q", "a", &[]).unwrap();
            let now = rappel_core::time::now_ms();
            for t in [now - 60_000, now - 30_000] {
                state
                    .record_review(&card.card.id, Grade::Pass, Duration::ZERO, t)
                    .unwrap();
            }
            let view = adw::NavigationView::new();
            let window = adw::Window::builder().content(&view).build();
            let root = adw::NavigationPage::builder().title("Root").build();
            view.push(&root);
            let page = ReviewPage::new(state.clone(), deck.clone(), |_| {}).unwrap();
            view.push(&page);
            window.present();
            let imp = page.imp();
            assert_eq!(imp.summary_page.title(), "Nothing to Study");
            assert!(imp.practice_button.get_visible());
            assert!(!imp.practice_button.has_css_class("suggested-action"));
            assert!(!imp.banner.is_revealed());
            page.activate_action("review.practice", None).unwrap();
            let practice = view.visible_page().and_downcast::<ReviewPage>().unwrap();
            assert!(practice.is_practice());
            assert_ne!(practice, page);
            assert_eq!(view.navigation_stack().n_items(), 2);
            assert_eq!(current(&practice), Some(card.card.id.clone()));

            // Nothing to practise: no button.
            state.set_suspended(&[card.card.id], true).unwrap();
            let page = ReviewPage::new(state.clone(), deck, |_| {}).unwrap();
            assert_eq!(page.imp().summary_page.title(), "Nothing to Study");
            assert!(!page.imp().practice_button.get_visible());
            window.close();
        });
    }

    #[test]
    fn practice_is_offered_only_with_cards_and_not_from_a_practice() {
        assert!(!practice_offered(0));
        assert!(practice_offered(1));
        assert!(practice_button_shown(false, true));
        assert!(!practice_button_shown(false, false));
        assert!(!practice_button_shown(true, true));
        assert_eq!(StartedFrom::DeckPage.done_label(), "_Back to Deck");
        assert_eq!(StartedFrom::AllCards.done_label(), "_Back to All Cards");
        assert_eq!(StartedFrom::DeckList.done_label(), "_Back to Decks");
    }

    /// Suspend during a practice goes through the normal path: the card
    /// leaves, nothing is counted, and its Undo brings it back next.
    #[test]
    fn a_card_suspended_during_a_practice_leaves_it_and_comes_back_by_undo() {
        crate::testing::with_widgets(|| {
            let (_tmp, state, page, _) = page_of(2);
            drop(page);
            let deck = state.decks().unwrap().remove(0);
            let page = practice_of(&state, &deck);
            let _window = in_window(&page);
            let first = current(&page).unwrap();
            page.remove(&first, Removal::Suspend);
            assert!(state.card(&first).unwrap().suspended);
            assert_ne!(current(&page).as_ref(), Some(&first));
            assert_eq!(page.imp().session.borrow().as_ref().unwrap().remaining(), 1);
            state
                .restore_suspended(std::slice::from_ref(&first), false)
                .unwrap();
            page.put_back(&first);
            assert_eq!(page.imp().session.borrow().as_ref().unwrap().remaining(), 2);
            assert_eq!(
                page.imp()
                    .session
                    .borrow()
                    .as_ref()
                    .unwrap()
                    .summary()
                    .cards,
                0
            );
        });
    }

    /// A tag's practice: its title names the tag as plain text, its cards
    /// come from every deck that is not archived, and a deck archived
    /// meanwhile leaves it, as in a tag's session.
    #[test]
    fn a_tag_practice_covers_its_decks_and_leaves_an_archived_one() {
        crate::testing::with_widgets(|| {
            let (_tmp, state) = crate::testing::temporary_state();
            let a = state.create_deck("A").unwrap();
            let b = state.create_deck("B").unwrap();
            let a1 = due_card(&state, &a.id, "a1", &["a&b <i>"]);
            let b1 = due_card(&state, &b.id, "b1", &["a&b <i>"]);
            let studied = Studied::Tag {
                tag: tag_named(&state, "a&b <i>"),
                from: StartedFrom::AllCards,
            };
            let page = ReviewPage::practice(state.clone(), studied, |_| {}).unwrap();
            let imp = page.imp();
            assert_eq!(imp.window_title.title(), "Tagged “\u{2068}a&b <i>\u{2069}”");
            assert_eq!(
                imp.done_button.label().as_deref(),
                Some("_Back to All Cards")
            );
            assert_eq!(imp.session.borrow().as_ref().unwrap().remaining(), 2);
            let on_screen = current(&page).unwrap();
            let deck = state.card(&on_screen).unwrap().deck_id;
            state.archive_deck(&deck).unwrap();
            page.leave_deck(&deck);
            let other = if on_screen == a1 { b1 } else { a1 };
            assert_eq!(current(&page), Some(other));
            pass(&page);
            assert_eq!(imp.summary_page.title(), "Practice Complete");
        });
    }

    /// Undo during a practice never brings back a card that can no longer
    /// be practised (deleted or suspended since): its answer is taken back
    /// silently and the step goes on to the answer before, or, with none,
    /// the card on screen stays. No error toast, nothing written.
    #[test]
    fn a_practice_undo_steps_over_cards_deleted_or_suspended_since() {
        crate::testing::with_widgets(|| {
            let (_tmp, state, page, _) = page_of(3);
            drop(page);
            let deck = state.decks().unwrap().remove(0);
            let answers = |page: &ReviewPage| {
                page.imp()
                    .session
                    .borrow()
                    .as_ref()
                    .unwrap()
                    .summary()
                    .answers
            };

            // The latest answer's card was deleted: it is stepped over and
            // the one before comes back.
            let page = practice_of(&state, &deck);
            let _window = in_window(&page);
            let first = current(&page).unwrap();
            pass(&page);
            let second = current(&page).unwrap();
            pass(&page);
            state.delete_card(&second).unwrap();
            assert_eq!(page.key(Key::Undo), glib::Propagation::Stop);
            assert_eq!(current(&page), Some(first.clone()));
            assert_eq!(answers(&page), 0);
            assert!(!can_undo(&page));
            assert!(page.imp().errors.is_empty(), "no error toast");
            state.undelete_card(&second).unwrap();

            // The only answer's card was suspended: Undo takes the answer
            // back, and the card on screen stays as it was.
            let page = practice_of(&state, &deck);
            let _window = in_window(&page);
            let answered = current(&page).unwrap();
            pass(&page);
            let on_screen = current(&page).unwrap();
            page.reveal();
            state
                .set_suspended(std::slice::from_ref(&answered), true)
                .unwrap();
            page.undo();
            assert_eq!(current(&page), Some(on_screen));
            assert!(page.imp().revealed.get(), "the card was not shown again");
            assert_eq!(answers(&page), 0);
            assert!(!can_undo(&page));
            assert!(page.imp().errors.is_empty(), "no error toast");
            // Nothing of it comes back later either.
            let mut rest = Vec::new();
            while let Some(id) = current(&page) {
                assert!(rest.len() < 10, "the practice does not end");
                rest.push(id);
                pass(&page);
            }
            assert!(!rest.contains(&answered));
        });
    }

    /// Undo does not bring back a card moved out of the practised deck
    /// since its answer, nor one that lost the practised tag: their answers
    /// are stepped over as for a deleted card.
    #[test]
    fn a_practice_undo_steps_over_cards_moved_or_untagged_since() {
        crate::testing::with_widgets(|| {
            let (_tmp, state, page, _) = page_of(3);
            drop(page);
            let deck = state.decks().unwrap().remove(0);
            let other = state.create_deck("Other").unwrap();
            let page = practice_of(&state, &deck);
            let _window = in_window(&page);
            let first = current(&page).unwrap();
            pass(&page);
            let second = current(&page).unwrap();
            pass(&page);
            let third = current(&page);
            state
                .move_cards(std::slice::from_ref(&second), &other.id)
                .unwrap();
            page.undo();
            assert_eq!(current(&page), Some(first));
            assert!(!can_undo(&page));
            assert!(page.imp().errors.is_empty(), "no error toast");
            let mut rest = Vec::new();
            while let Some(id) = current(&page) {
                assert!(rest.len() < 10, "the practice does not end");
                rest.push(id);
                pass(&page);
            }
            assert!(!rest.contains(&second));
            assert!(third.is_some_and(|t| rest.contains(&t)));

            // A tag's practice: a card untagged since is stepped over.
            let a = due_card(&state, &other.id, "t1", &["t"]);
            let b = due_card(&state, &other.id, "t2", &["t"]);
            let studied = Studied::Tag {
                tag: tag_named(&state, "t"),
                from: StartedFrom::AllCards,
            };
            let page = ReviewPage::practice(state.clone(), studied, |_| {}).unwrap();
            let _window = in_window(&page);
            let answered = current(&page).unwrap();
            pass(&page);
            let on_screen = current(&page).unwrap();
            assert!([&a, &b].contains(&&answered));
            let card = state.card(&answered).unwrap();
            state
                .edit_card_with_tags(&answered, &card.front_md, &card.back_md, Some(&[]))
                .unwrap();
            page.undo();
            assert_eq!(current(&page), Some(on_screen));
            assert!(!can_undo(&page));
        });
    }
}
