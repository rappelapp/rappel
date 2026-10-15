//! A study session of one deck: the front, then the back, then Fail or
//! Pass; a summary at the end. Layout in `data/review_page.blp` (ADR 0014).
//!
//! The order of cards is the core's [`Session`]. Each answer is recorded
//! at once with `record_review`, so leaving early loses nothing.
//!
//! Keys, in the bubble phase so a focused button keeps its own Space and
//! Enter: Space or Enter shows the answer; then 1 is Fail, 2 or Space is
//! Pass. The focus rests on the card between answers.

use std::time::{Duration, Instant};

use adw::prelude::*;
use adw::subclass::prelude::*;
use cards_core::domain::{CardState, Deck, Grade};
use cards_core::scheduling::session::{Session, Summary, capped};
use gtk::glib;

use crate::clock::local_day_start_ms;
use crate::state::AppState;
use crate::ui::toast::{log_failure, persistent_toast, show_toast_from};
use crate::ui::{init_once, nav};

mod imp {
    use std::cell::{Cell, OnceCell, RefCell};
    use std::time::Instant;

    use adw::prelude::*;
    use adw::subclass::prelude::*;
    use cards_core::domain::Deck;
    use cards_core::scheduling::session::Session;
    use gtk::glib;

    use crate::state::AppState;

    #[derive(Debug, Default, gtk::CompositeTemplate)]
    #[template(resource = "/io/github/rsmirke/Cards/review_page.ui")]
    pub struct ReviewPage {
        #[template_child]
        pub window_title: TemplateChild<adw::WindowTitle>,
        #[template_child]
        pub stack: TemplateChild<gtk::Stack>,
        #[template_child]
        pub card_area: TemplateChild<gtk::Box>,
        #[template_child]
        pub front_label: TemplateChild<gtk::Label>,
        #[template_child]
        pub back_separator: TemplateChild<gtk::Separator>,
        #[template_child]
        pub back_label: TemplateChild<gtk::Label>,
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
        /// When the current card's front was shown.
        pub shown_at: Cell<Option<Instant>>,
        pub revealed: Cell<bool>,
        /// When the back was shown.
        pub revealed_at: Cell<Option<Instant>>,
        /// The last error toast, replaced rather than stacked when saving
        /// keeps failing.
        pub error_toast: RefCell<Option<adw::Toast>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for ReviewPage {
        const NAME: &'static str = "CardsReviewPage";
        type Type = super::ReviewPage;
        type ParentType = adw::NavigationPage;

        fn class_init(klass: &mut Self::Class) {
            klass.bind_template();
            klass.install_action("review.reveal", None, |page, _, _| page.reveal());
            klass.install_action("review.fail", None, |page, _, _| {
                page.grade(cards_core::domain::Grade::Fail);
            });
            klass.install_action("review.pass", None, |page, _, _| {
                page.grade(cards_core::domain::Grade::Pass);
            });
            klass.install_action("review.done", None, |page, _, _| page.done());
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
#[derive(Debug, Clone, Copy)]
enum Key {
    /// Space: show the answer, or Pass once it is shown.
    Space,
    /// Enter: show the answer.
    Enter,
    Fail,
    Pass,
}

impl ReviewPage {
    /// A session over what `deck` has to study now, showing its first card.
    ///
    /// # Errors
    /// If what to study cannot be read.
    pub fn new(state: AppState, deck: Deck) -> cards_core::Result<Self> {
        let now = cards_core::time::now_ms();
        let input = state.study_input(&deck.id, now, local_day_start_ms(now))?;
        let page: Self = glib::Object::new();
        let imp = page.imp();
        // Deck names are untrusted; AdwWindowTitle shows plain text.
        imp.window_title.set_title(&deck.name);
        imp.session.replace(Some(Session::new(input)));
        init_once(&imp.state, state);
        init_once(&imp.deck, deck);
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
        if imp.current.borrow().is_none() {
            return glib::Propagation::Proceed;
        }
        // Holding Space a moment too long repeats it: a reveal must not
        // become a Pass by itself, since a grade cannot be undone yet.
        let settled = imp
            .revealed_at
            .get()
            .is_some_and(|t| t.elapsed() >= KEY_SETTLE);
        match (key, imp.revealed.get()) {
            (Key::Space | Key::Enter, false) => self.reveal(),
            (Key::Space | Key::Pass | Key::Fail, true) if !settled => {}
            (Key::Space | Key::Pass, true) => self.grade(Grade::Pass),
            (Key::Fail, true) => self.grade(Grade::Fail),
            // 1 and 2 before the answer, Enter after it: nothing.
            (Key::Fail | Key::Pass, false) | (Key::Enter, true) => {}
        }
        glib::Propagation::Stop
    }

    /// Shows the next card, or the summary when the session is over.
    fn show_next(&self) {
        let imp = self.imp();
        let Some(state) = imp.state.get() else {
            return;
        };
        let now = cards_core::time::now_ms();
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
        imp.front_label.set_label(&card.front_md);
        imp.back_label.set_label(&card.back_md);
        imp.back_label.set_visible(false);
        imp.back_separator.set_visible(false);
        imp.revealed.set(false);
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
        imp.back_separator.set_visible(true);
        imp.back_label.set_visible(true);
        self.set_controls(Controls::Grade);
        imp.card_area.grab_focus();
    }

    /// Shows `controls` in the bottom bar and enables only their actions.
    fn set_controls(&self, controls: Controls) {
        let (name, reveal, grade) = match controls {
            Controls::Reveal => ("reveal", true, false),
            Controls::Grade => ("grade", false, true),
            Controls::Hidden => ("none", false, false),
        };
        self.imp().buttons.set_visible_child_name(name);
        self.action_set_enabled("review.reveal", reveal);
        self.action_set_enabled("review.fail", grade);
        self.action_set_enabled("review.pass", grade);
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
        let now = cards_core::time::now_ms();
        if let Err(err) = state.record_review(&id, grade, duration, now) {
            self.error_toast(&format!("The answer could not be saved: {err}"));
            return;
        }
        let card_state = state.card_state(&id).unwrap_or_else(|err| {
            // The answer is saved; without its new state the card just
            // does not come back in this session.
            tracing::warn!("cannot read a card's state after an answer: {err}");
            CardState::New
        });
        if let Some(session) = imp.session.borrow_mut().as_mut() {
            session.graded(&id, grade, duration, &card_state);
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
        log_failure("a review operation", message);
        let toast = persistent_toast(message);
        if let Some(old) = self.imp().error_toast.replace(Some(toast.clone())) {
            old.dismiss();
        }
        show_toast_from(self, toast);
    }
}

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
    use super::*;

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
