//! The card editor: front and back as plain text. Layout in
//! `data/card_editor.blp` (ADR 0013).
//!
//! - Ctrl+Enter saves and closes; Ctrl+Shift+Enter saves and starts a new
//!   card in the same deck, with the focus in the front field. Both are
//!   caught in the capture phase, before the text views see the Enter.
//! - A blank front disables saving, and so does a side over the store's
//!   length limit, explained under that side (the store's own rules).
//! - Closing with unsaved changes asks first, whether by Escape, Cancel or
//!   closing the window (`CardsWindow` calls [`CardEditor::confirm_discard`]).
//! - A store error keeps the dialog open with the text as typed.

use std::rc::Rc;

use adw::prelude::*;
use adw::subclass::prelude::*;
use cards_core::domain::{Card, CardSide, MAX_CARD_SIDE_BYTES, card_side_fits};
use gtk::glib;

use crate::state::AppState;
use crate::ui::callback::Callback;
use crate::ui::text::group_digits;
use crate::ui::toast::{log_failure, persistent_toast};
use crate::ui::{dialog, init_once};

/// What a save did, for the deck page to update its list.
#[derive(Debug, Clone)]
pub enum Saved {
    Added(Card),
    Edited(Card),
}

type OnSaved = dyn Fn(Saved);

mod imp {
    use std::cell::{Cell, OnceCell, RefCell};

    use adw::prelude::*;
    use adw::subclass::prelude::*;
    use gtk::glib;

    use super::OnSaved;
    use crate::state::AppState;
    use crate::ui::callback::Callback;

    #[derive(Default, gtk::CompositeTemplate)]
    #[template(resource = "/io/github/rsmirke/Cards/card_editor.ui")]
    pub struct CardEditor {
        #[template_child]
        pub toast_overlay: TemplateChild<adw::ToastOverlay>,
        #[template_child]
        pub front_view: TemplateChild<gtk::TextView>,
        #[template_child]
        pub back_view: TemplateChild<gtk::TextView>,
        #[template_child]
        pub front_error: TemplateChild<gtk::Label>,
        #[template_child]
        pub back_error: TemplateChild<gtk::Label>,
        pub state: OnceCell<AppState>,
        pub deck_id: OnceCell<String>,
        /// The card being edited; `None` for a new card.
        pub card_id: RefCell<Option<String>>,
        /// Front and back as last loaded or saved, to tell unsaved changes.
        pub saved_text: RefCell<(String, String)>,
        pub on_saved: OnceCell<Callback<OnSaved>>,
        /// The "Discard Changes?" question is showing; it is not asked twice.
        pub asking_discard: Cell<bool>,
        /// Discard was chosen; the editor is closing without saving.
        pub discarded: Cell<bool>,
    }

    /// Written by hand to leave out the text fields: card content is never
    /// logged above debug level (SPEC §2), and `{:?}` would print it.
    impl std::fmt::Debug for CardEditor {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.debug_struct("CardEditor")
                .field("deck_id", &self.deck_id)
                .field("card_id", &self.card_id)
                .finish_non_exhaustive()
        }
    }

    #[glib::object_subclass]
    impl ObjectSubclass for CardEditor {
        const NAME: &'static str = "CardsCardEditor";
        type Type = super::CardEditor;
        type ParentType = adw::Dialog;

        fn class_init(klass: &mut Self::Class) {
            klass.bind_template();
            klass.install_action("editor.save", None, |editor, _, _| {
                editor.save(false);
            });
            klass.install_action("editor.save-and-new", None, |editor, _, _| {
                editor.save(true);
            });
            klass.install_action("editor.cancel", None, |editor, _, _| {
                editor.close();
            });
        }

        fn instance_init(obj: &glib::subclass::InitializingObject<Self>) {
            obj.init_template();
        }
    }

    impl ObjectImpl for CardEditor {
        fn constructed(&self) {
            self.parent_constructed();
            self.obj().setup();
        }
    }
    impl WidgetImpl for CardEditor {}
    impl AdwDialogImpl for CardEditor {}
}

glib::wrapper! {
    pub struct CardEditor(ObjectSubclass<imp::CardEditor>)
        @extends adw::Dialog, gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget, gtk::ShortcutManager;
}

impl CardEditor {
    /// An editor for a new card in `deck_id` (`card` is `None`) or for
    /// `card`. `on_saved` is called after each successful save.
    pub fn new(
        state: AppState,
        deck_id: &str,
        card: Option<&Card>,
        on_saved: impl Fn(Saved) + 'static,
    ) -> Self {
        let editor: Self = glib::Object::new();
        let imp = editor.imp();
        init_once(&imp.state, state);
        init_once(&imp.deck_id, deck_id.to_owned());
        init_once(&imp.on_saved, Callback::new(Rc::new(on_saved)));
        match card {
            Some(card) => editor.load(Some(&card.id), &card.front_md, &card.back_md),
            None => editor.load(None, "", ""),
        }
        editor
    }

    /// Shows the editor over `parent`.
    pub fn show(&self, parent: &impl IsA<gtk::Widget>) {
        self.present(Some(parent));
        self.set_focus(Some(&*self.imp().front_view));
    }

    fn setup(&self) {
        let imp = self.imp();
        // Capture phase: the text views would otherwise take the Enter.
        let shortcuts = gtk::ShortcutController::new();
        shortcuts.set_propagation_phase(gtk::PropagationPhase::Capture);
        for (trigger, and_new) in [
            ("<Control>Return", false),
            ("<Control>KP_Enter", false),
            ("<Control><Shift>Return", true),
            ("<Control><Shift>KP_Enter", true),
        ] {
            // Always handled, even with a blank front, so the key never
            // falls through to the text view as a new line.
            let action = gtk::CallbackAction::new(move |widget, _| {
                if let Some(editor) = widget.downcast_ref::<Self>() {
                    editor.save(and_new);
                }
                glib::Propagation::Stop
            });
            shortcuts.add_shortcut(gtk::Shortcut::new(
                gtk::ShortcutTrigger::parse_string(trigger),
                Some(action),
            ));
        }
        self.add_controller(shortcuts);

        for view in [&*imp.front_view, &*imp.back_view] {
            view.buffer().connect_changed(glib::clone!(
                #[weak(rename_to = editor)]
                self,
                move |_| editor.update_actions()
            ));
        }
        self.connect_close_attempt(Self::ask_discard);
    }

    /// Shows `front` and `back` as the saved state of card `card_id`.
    fn load(&self, card_id: Option<&str>, front: &str, back: &str) {
        let imp = self.imp();
        self.set_title(if card_id.is_some() {
            "Edit Card"
        } else {
            "New Card"
        });
        imp.card_id.replace(card_id.map(str::to_owned));
        imp.saved_text.replace((front.to_owned(), back.to_owned()));
        imp.front_view.buffer().set_text(front);
        imp.back_view.buffer().set_text(back);
        self.update_actions();
    }

    fn texts(&self) -> (String, String) {
        let text = |view: &gtk::TextView| {
            let buffer = view.buffer();
            let (start, end) = buffer.bounds();
            buffer.text(&start, &end, false).to_string()
        };
        let imp = self.imp();
        (text(&imp.front_view), text(&imp.back_view))
    }

    fn update_actions(&self) {
        let imp = self.imp();
        let texts = self.texts();
        let check = check_card(&texts.0, &texts.1);
        self.action_set_enabled("editor.save", check.can_save);
        self.action_set_enabled("editor.save-and-new", check.can_save);
        show_error(&imp.front_view, &imp.front_error, check.front_error);
        show_error(&imp.back_view, &imp.back_error, check.back_error);
        // With unsaved changes, closing emits `close-attempt` instead.
        self.set_can_close(texts == *self.imp().saved_text.borrow());
    }

    fn save(&self, and_new: bool) {
        let imp = self.imp();
        let (front, back) = self.texts();
        if !check_card(&front, &back).can_save {
            return;
        }
        let (Some(state), Some(deck_id)) = (imp.state.get(), imp.deck_id.get()) else {
            return;
        };
        let card_id = imp.card_id.borrow().clone();
        // An unchanged card is not written (that would bump `updated_at`).
        if card_id.is_some() && (front.clone(), back.clone()) == *imp.saved_text.borrow() {
            self.after_save(and_new);
            return;
        }
        let result = match &card_id {
            Some(id) => state.edit_card(id, &front, &back).map(Saved::Edited),
            None => state.add_card(deck_id, &front, &back).map(Saved::Added),
        };
        let saved = match result {
            Ok(saved) => saved,
            Err(err) => {
                // The editor's own overlay: the dialog covers the page's.
                let message = format!("The card could not be saved: {err}");
                log_failure("a card save", &message);
                imp.toast_overlay.add_toast(persistent_toast(&message));
                return;
            }
        };
        let toast_text = match &saved {
            Saved::Added(_) => "Card added",
            Saved::Edited(_) => "Card saved",
        };
        if let Some(on_saved) = imp.on_saved.get().map(Callback::get) {
            on_saved(saved);
        }
        self.after_save(and_new);
        if and_new {
            imp.toast_overlay.add_toast(
                adw::Toast::builder()
                    .title(toast_text)
                    .use_markup(false)
                    .build(),
            );
        }
    }

    /// After a save (or nothing to save): an empty editor for the next
    /// card, or close.
    fn after_save(&self, and_new: bool) {
        if and_new {
            self.load(None, "", "");
            self.imp().front_view.grab_focus();
        } else {
            self.force_close();
        }
    }

    fn ask_discard(&self) {
        glib::spawn_future_local(glib::clone!(
            #[weak(rename_to = editor)]
            self,
            async move {
                if editor.confirm_discard().await {
                    editor.discard();
                }
            }
        ));
    }

    /// Whether the editor holds text that is not saved and not discarded.
    pub fn has_unsaved_changes(&self) -> bool {
        !self.imp().discarded.get() && !self.can_close()
    }

    /// Closes without saving, after the user chose Discard.
    pub fn discard(&self) {
        self.imp().discarded.set(true);
        self.force_close();
    }

    /// Asks "Discard Changes?" and returns whether the user chose Discard.
    /// Returns `false` at once if the question is already showing, so the
    /// caller that asked first decides.
    pub async fn confirm_discard(&self) -> bool {
        let asking = &self.imp().asking_discard;
        if asking.replace(true) {
            return false;
        }
        let discard = match dialog::alert("discard_dialog.ui", "discard_dialog") {
            Some(dialog) => dialog.choose_future(Some(self)).await == "discard",
            None => false,
        };
        asking.set(false);
        discard
    }
}

/// What the editor shows for the texts typed.
#[derive(Debug, Clone, PartialEq, Eq)]
struct CardCheck {
    /// Whether Save is enabled.
    can_save: bool,
    /// The error under the front, if any.
    front_error: Option<String>,
    /// The error under the back, if any.
    back_error: Option<String>,
}

/// The store's rules, checked as the user types: a blank front cannot be
/// saved (no error is shown, the empty field says it), nor can a side over
/// the length limit, which is explained under it.
fn check_card(front: &str, back: &str) -> CardCheck {
    let too_long = |text: &str, side| (!card_side_fits(text)).then(|| too_long_text(side));
    let front_error = too_long(front, CardSide::Front);
    let back_error = too_long(back, CardSide::Back);
    CardCheck {
        can_save: !front.trim().is_empty() && front_error.is_none() && back_error.is_none(),
        front_error,
        back_error,
    }
}

fn too_long_text(side: CardSide) -> String {
    format!(
        "The {side} is too long (the limit is {} bytes)",
        group_digits(MAX_CARD_SIDE_BYTES)
    )
}

/// Shows `error` under `view`, announcing it when it appears or changes.
fn show_error(view: &gtk::TextView, label: &gtk::Label, error: Option<String>) {
    match error {
        Some(message) => {
            if !label.is_visible() || label.label() != message {
                view.announce(&message, gtk::AccessibleAnnouncementPriority::Medium);
            }
            label.set_label(&message);
            label.set_visible(true);
        }
        None => label.set_visible(false),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_blank_front_cannot_be_saved() {
        for front in ["", " \n\t "] {
            let check = check_card(front, "back");
            assert!(!check.can_save);
            assert_eq!((check.front_error, check.back_error), (None, None));
        }
        assert!(check_card(" x ", "").can_save);
    }

    #[test]
    fn sides_at_the_limit_can_be_saved() {
        let longest = "ü".repeat(MAX_CARD_SIDE_BYTES / 2);
        let check = check_card(&longest, &longest);
        assert!(check.can_save);
        assert_eq!((check.front_error, check.back_error), (None, None));
    }

    #[test]
    fn a_side_over_the_limit_is_explained_under_it() {
        let over = "x".repeat(MAX_CARD_SIDE_BYTES + 1);
        let message = |side| {
            Some(format!(
                "The {side} is too long (the limit is 1,048,576 bytes)"
            ))
        };

        let check = check_card(&over, "back");
        assert!(!check.can_save);
        assert_eq!(check.front_error, message("front"));
        assert_eq!(check.back_error, None);

        let check = check_card("front", &over);
        assert!(!check.can_save);
        assert_eq!(check.front_error, None);
        assert_eq!(check.back_error, message("back"));

        let check = check_card(&over, &over);
        assert_eq!(
            (check.front_error, check.back_error),
            (message("front"), message("back"))
        );
    }
}
