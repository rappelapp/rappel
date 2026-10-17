//! A study session of one deck: the front, then the back, then Fail or
//! Pass; a summary at the end. Layout in `data/review_page.blp` (ADR 0014).
//!
//! The order of cards is the core's [`Session`]. Each answer is recorded
//! at once with `record_review`, so leaving early loses nothing.
//!
//! Keys, in the bubble phase so a focused button keeps its own Space and
//! Enter: Space or Enter shows the answer; then 1 is Fail, 2 or Space is
//! Pass; E edits the card at any time; Ctrl+Z (and the header's Undo button)
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
use rappel_core::domain::{CardState, Deck, Grade};
use rappel_core::scheduling::{Session, Summary, capped};

use crate::dialogs::card_editor::{CardEditor, Saved};
use crate::render::{back_shown, clear, fill, shown_part};
use crate::state::AppState;
use crate::ui::callback::Callback;
use crate::ui::toast::{info_toast, show_toast_from};
use crate::ui::undo::{self, UndoFailure};
use crate::ui::{init_once, nav};

type OnSaved = dyn Fn(Saved);

mod imp {
    use std::cell::{Cell, OnceCell, RefCell};
    use std::rc::Rc;
    use std::time::Instant;

    use adw::prelude::*;
    use adw::subclass::prelude::*;
    use gtk::glib;
    use rappel_core::content::Document;
    use rappel_core::domain::Deck;
    use rappel_core::scheduling::Session;

    use super::OnSaved;
    use crate::state::AppState;
    use crate::ui::callback::Callback;
    use crate::ui::toast::ErrorSlot;

    #[derive(Debug, Default, gtk::CompositeTemplate)]
    #[template(resource = "/io/github/rsmirke/Rappel/review_page.ui")]
    pub struct ReviewPage {
        #[template_child]
        pub window_title: TemplateChild<adw::WindowTitle>,
        #[template_child]
        pub edit_button: TemplateChild<gtk::Button>,
        #[template_child]
        pub stack: TemplateChild<gtk::Stack>,
        #[template_child]
        pub card_area: TemplateChild<gtk::Box>,
        #[template_child]
        pub front_box: TemplateChild<gtk::Box>,
        #[template_child]
        pub back_separator: TemplateChild<gtk::Separator>,
        #[template_child]
        pub back_box: TemplateChild<gtk::Box>,
        #[template_child]
        pub cut_note: TemplateChild<gtk::Label>,
        #[template_child]
        pub buttons: TemplateChild<gtk::Stack>,
        #[template_child]
        pub summary_page: TemplateChild<adw::StatusPage>,
        #[template_child]
        pub done_button: TemplateChild<gtk::Button>,
        pub state: OnceCell<AppState>,
        pub deck: OnceCell<Deck>,
        pub session: RefCell<Option<Session>>,
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
        /// Told about every save made from this page's editor.
        pub on_saved: OnceCell<Callback<OnSaved>>,
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
            klass.install_action("review.undo", None, |page, _, _| page.undo());
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
    /// Ctrl+Z: take back the last answer.
    Undo,
}

/// What a key makes the page do.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Act {
    Reveal,
    Grade(Grade),
    Edit,
    Undo,
}

/// What `key` does when the answer is (not) `revealed`. `settled` says
/// whether [`KEY_SETTLE`] has passed since the reveal. Holding Space a
/// moment too long repeats it: a reveal must not become a Pass by itself,
/// even though a grade can be undone. E and Ctrl+Z are never delayed: they
/// grade nothing.
fn key_action(key: Key, revealed: bool, settled: bool) -> Option<Act> {
    match (key, revealed) {
        (Key::Edit, _) => Some(Act::Edit),
        (Key::Undo, _) => Some(Act::Undo),
        (Key::Space | Key::Enter, false) => Some(Act::Reveal),
        (Key::Space | Key::Pass | Key::Fail, true) if !settled => None,
        (Key::Space | Key::Pass, true) => Some(Act::Grade(Grade::Pass)),
        (Key::Fail, true) => Some(Act::Grade(Grade::Fail)),
        // 1 and 2 before the answer, Enter after it: nothing.
        (Key::Fail | Key::Pass, false) | (Key::Enter, true) => None,
    }
}

impl ReviewPage {
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
        let now = rappel_core::time::now_ms();
        let input = state.study_input(&deck.id, now)?;
        let page: Self = glib::Object::new();
        let imp = page.imp();
        // Deck names are untrusted; AdwWindowTitle shows plain text.
        imp.window_title.set_title(&deck.name);
        imp.session.replace(Some(Session::new(input)));
        init_once(&imp.state, state);
        init_once(&imp.deck, deck);
        init_once(&imp.on_saved, Callback::new(Rc::new(on_saved)));
        page.show_next();
        Ok(page)
    }

    fn setup_keys(&self) {
        let shortcuts = gtk::ShortcutController::new();
        // Bubble: a focused button handles its own Space and Enter first.
        shortcuts.set_propagation_phase(gtk::PropagationPhase::Bubble);
        for (trigger, key) in [
            ("space", Key::Space),
            ("Return", Key::Enter),
            ("KP_Enter", Key::Enter),
            ("1", Key::Fail),
            ("KP_1", Key::Fail),
            ("2", Key::Pass),
            ("KP_2", Key::Pass),
            ("e", Key::Edit),
            ("<Shift>e", Key::Edit),
            ("<Control>z", Key::Undo),
        ] {
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
        if imp.editing.get() {
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
                self.error_toast(&format!("A card could not be read and was skipped: {err}"));
                if let Some(session) = imp.session.borrow_mut().as_mut() {
                    session.skip(&id);
                }
                self.show_next();
                return;
            }
        };
        imp.revealed.set(false);
        self.show_front(&card.front_md);
        clear(&imp.back_box);
        imp.back_box.set_visible(false);
        imp.back_separator.set_visible(false);
        imp.card_area
            .reset_property(gtk::AccessibleProperty::Description);
        self.keep_back(&card.back_md);
        imp.revealed_at.set(None);
        imp.current.replace(Some(id));
        imp.shown_at.set(Some(Instant::now()));
        let left = imp.session.borrow().as_ref().map_or(0, Session::remaining);
        imp.window_title.set_subtitle(&left_text(left));
        self.set_controls(Controls::Reveal);
        imp.stack.set_visible_child_name("card");
        imp.card_area.grab_focus();
    }

    fn reveal(&self) {
        let imp = self.imp();
        if imp.current.borrow().is_none() || imp.revealed.replace(true) {
            return;
        }
        imp.revealed_at.set(Some(Instant::now()));
        let back = imp.back.take();
        self.show_back(&back);
        self.set_controls(Controls::Grade);
        imp.card_area.grab_focus();
    }

    /// Shows the front `front_md`, or its start if it is too long to lay
    /// out (`render::shown_part`). Screen readers read it when the card
    /// area has the focus.
    fn show_front(&self, front_md: &str) {
        let imp = self.imp();
        let (part, cut) = shown_part(front_md);
        let front = content::parse(part);
        fill(&imp.front_box, &front);
        imp.card_area
            .update_property(&[gtk::accessible::Property::Label(&with_cut_note(
                content::plain_text(&front),
                cut,
            ))]);
        imp.front_cut.set(cut);
        self.update_cut_note();
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
    /// only the grade buttons appear.
    fn show_back(&self, back: &Document) {
        let imp = self.imp();
        let shown = back_shown(back);
        if shown {
            fill(&imp.back_box, back);
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
        imp.back_separator.set_visible(shown);
        imp.back_box.set_visible(shown);
        self.update_cut_note();
    }

    /// Opens the card editor on the card on screen.
    fn edit(&self) {
        let imp = self.imp();
        let (Some(state), Some(deck), Some(id)) = (
            imp.state.get(),
            imp.deck.get(),
            imp.current.borrow().clone(),
        ) else {
            return;
        };
        if imp.editing.get() {
            return;
        }
        // Read again, so the editor starts from what is stored.
        let card = match state.card(&id) {
            Ok(card) => card,
            Err(err) => {
                self.error_toast(&format!("The card could not be read: {err}"));
                return;
            }
        };
        CardEditor::open(
            self,
            &imp.editing,
            state.clone(),
            &deck.id,
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
            self.show_back(&back);
        }
    }

    /// Shows `controls` in the bottom bar and enables only their actions.
    fn set_controls(&self, controls: Controls) {
        let (name, reveal, grade) = match controls {
            Controls::Reveal => ("reveal", true, false),
            Controls::Grade => ("grade", false, true),
            Controls::Hidden => ("none", false, false),
        };
        // Hidden with the summary, where there is no card to edit.
        let editable = !matches!(controls, Controls::Hidden);
        self.action_set_enabled("review.edit", editable);
        self.imp().edit_button.set_visible(editable);
        self.imp().buttons.set_visible_child_name(name);
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
            .is_some_and(|s| s.last_answer().is_some());
        self.action_set_enabled("review.undo", can_undo);
    }

    /// Takes back the last answer of the session: the log first, then the
    /// session, then the card is shown again as new (front side, timer
    /// restarted), whether a card or the summary was on screen.
    fn undo(&self) {
        let imp = self.imp();
        let Some(state) = imp.state.get() else {
            return;
        };
        let last = imp
            .session
            .borrow()
            .as_ref()
            .and_then(|s| s.last_answer().map(|(c, e)| (c.to_owned(), e.to_owned())));
        let Some((card_id, entry_id)) = last else {
            return;
        };
        match state.undo(&card_id, &entry_id, rappel_core::time::now_ms()) {
            Ok(_) => {
                if let Some(session) = imp.session.borrow_mut().as_mut() {
                    session.undo_last();
                }
                imp.current.replace(None);
                self.show_next();
            }
            Err(err) => match undo::failure(&err) {
                UndoFailure::Stale => {
                    // Older answers are undone only after this one, so none
                    // of them can be now.
                    if let Some(session) = imp.session.borrow_mut().as_mut() {
                        session.forget_answers();
                    }
                    self.update_undo();
                    // The button may have had the focus and is now off:
                    // put it where the page's keys work.
                    self.park_focus();
                    show_toast_from(self, info_toast("This answer can no longer be undone"));
                }
                UndoFailure::Failed => {
                    self.error_toast(&format!("The answer could not be undone: {err}"));
                }
            },
        }
    }

    /// Records the answer for the card on screen and shows the next one.
    /// If it cannot be recorded, the card stays so the answer can be given
    /// again.
    fn grade(&self, grade: Grade) {
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
                self.error_toast(&format!("The answer could not be saved: {err}"));
                return;
            }
        };
        let card_state = state.card_state(&id).unwrap_or_else(|err| {
            // The answer is saved; without its new state the card just
            // does not come back in this session.
            tracing::warn!("cannot read a card's state after an answer: {err}");
            CardState::New
        });
        if let Some(session) = imp.session.borrow_mut().as_mut() {
            session.graded(&id, &event.id, grade, duration, &card_state);
        }
        imp.current.replace(None);
        self.show_next();
    }

    fn show_summary(&self) {
        let imp = self.imp();
        imp.current.replace(None);
        imp.shown_at.set(None);
        let summary = imp
            .session
            .borrow()
            .as_ref()
            .map(Session::summary)
            .unwrap_or_default();
        // Our own text, but the property is markup.
        imp.summary_page
            .set_description(Some(&glib::markup_escape_text(&summary_text(&summary))));
        imp.window_title.set_subtitle("");
        self.set_controls(Controls::Hidden);
        imp.stack.set_visible_child_name("summary");
        imp.done_button.grab_focus();
    }

    fn done(&self) {
        nav::pop(self);
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
        format!("{text}\n\n{CUT_NOTE}")
    } else {
        text
    }
}

/// The note's text for screen readers. Keep it equal to `cut_note` in
/// `review_page.blp`.
const CUT_NOTE: &str = "This card is long: only its start is shown. Edit it to see all of it.";

/// The subtitle while studying: "12 left". It counts the card on screen
/// and step cards that will come back, so it can go up after a Fail.
fn left_text(left: usize) -> String {
    format!("{left} left")
}

/// The summary: "12 cards · 83% passed · 4 min 10 s".
fn summary_text(summary: &Summary) -> String {
    let cards = match summary.cards {
        1 => "1 card".to_owned(),
        n => format!("{n} cards"),
    };
    let mut parts = vec![cards];
    if let Some(percent) = summary.pass_percent() {
        parts.push(format!("{percent}% passed"));
    }
    parts.push(duration_text(summary.time));
    parts.join(" · ")
}

/// "45 s", "4 min 10 s", "1 h 2 min" (seconds rounded down).
fn duration_text(time: Duration) -> String {
    let secs = time.as_secs();
    let (h, m, s) = (secs / 3600, secs / 60 % 60, secs % 60);
    match (h, m) {
        (0, 0) => format!("{s} s"),
        (0, m) => format!("{m} min {s} s"),
        (h, m) => format!("{h} h {m} min"),
    }
}

#[cfg(test)]
mod tests {
    use rappel_core::domain::EventKind;

    use super::*;

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
            format!("Front\n\n{CUT_NOTE}")
        );
    }

    #[test]
    fn the_cut_note_matches_the_blueprint() {
        let blueprint = include_str!("../../data/review_page.blp");
        assert!(blueprint.contains(&format!("_(\"{CUT_NOTE}\")")));
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
        // E edits at any time, settled or not.
        for (revealed, settled) in [(false, false), (true, false), (true, true)] {
            assert_eq!(key_action(Edit, revealed, settled), Some(Act::Edit));
        }
        // Ctrl+Z too, and it grades nothing, so key repeat does no harm.
        for (revealed, settled) in [(false, false), (true, false), (true, true)] {
            assert_eq!(key_action(Key::Undo, revealed, settled), Some(Act::Undo));
        }
    }

    /// A page over a deck of `n` cards that are all due, with the app state.
    fn page_of(n: usize) -> (tempfile::TempDir, AppState, ReviewPage, Vec<String>) {
        adw::init().unwrap();
        gtk::gio::resources_register_include!("rappel.gresource").unwrap();
        let tmp = tempfile::tempdir().unwrap();
        let state = AppState::open(tmp.path().join("data")).unwrap();
        let deck = state.create_deck("Deck").unwrap();
        let now = rappel_core::time::now_ms();
        let day = 24 * 60 * 60 * 1000;
        let ids: Vec<String> = (0..n)
            .map(|i| {
                let id = state
                    .add_card_with_tags(&deck.id, &format!("front {i}"), "back", &[])
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
        (tmp, state, page, ids)
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

    fn can_undo(page: &ReviewPage) -> bool {
        page.imp()
            .session
            .borrow()
            .as_ref()
            .is_some_and(|s| s.last_answer().is_some())
    }

    fn undo_entries(state: &AppState, card: &str) -> usize {
        let log = state.review_events(card).unwrap();
        log.iter()
            .filter(|e| matches!(e.kind, EventKind::Undo { .. }))
            .count()
    }

    #[test]
    fn undo_goes_back_to_the_first_answer() {
        crate::testing::with_gtk(|| {
            let (_tmp, state, page, ids) = page_of(3);
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
        crate::testing::with_gtk(|| {
            let (_tmp, state, page, ids) = page_of(2);
            pass(&page);
            pass(&page);
            let imp = page.imp();
            assert_eq!(imp.stack.visible_child_name().as_deref(), Some("summary"));
            assert!(current(&page).is_none());
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
        crate::testing::with_gtk(|| {
            let (_tmp, state, page, ids) = page_of(2);
            pass(&page);
            state
                .edit_card_with_tags(&ids[0], "edited front", "back", None)
                .unwrap();
            page.undo();
            let front = first_label(page.imp().front_box.upcast_ref());
            assert_eq!(front.as_deref(), Some("edited front"));
        });
    }

    #[test]
    fn rapid_repeated_undo_stops_at_the_first_answer() {
        crate::testing::with_gtk(|| {
            let (_tmp, state, page, ids) = page_of(3);
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
        crate::testing::with_gtk(|| {
            let (_tmp, state, page, ids) = page_of(2);
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
    fn durations_read_naturally() {
        assert_eq!(duration_text(Duration::ZERO), "0 s");
        assert_eq!(duration_text(Duration::from_millis(45_900)), "45 s");
        assert_eq!(duration_text(Duration::from_secs(250)), "4 min 10 s");
        assert_eq!(duration_text(Duration::from_secs(3_600 + 125)), "1 h 2 min");
    }

    #[test]
    fn summary_lists_cards_rate_and_time() {
        let summary = Summary {
            cards: 12,
            answers: 18,
            passes: 15,
            time: Duration::from_secs(250),
        };
        assert_eq!(summary_text(&summary), "12 cards · 83% passed · 4 min 10 s");
        let one = Summary {
            cards: 1,
            answers: 1,
            passes: 0,
            time: Duration::from_secs(3),
        };
        assert_eq!(summary_text(&one), "1 card · 0% passed · 3 s");
        assert_eq!(summary_text(&Summary::default()), "0 cards · 0 s");
        assert_eq!(left_text(3), "3 left");
    }
}
