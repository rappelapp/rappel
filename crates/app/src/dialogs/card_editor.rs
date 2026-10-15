//! The card editor: front and back as plain text. Layout in
//! `data/card_editor.blp` (ADR 0013).
//!
//! - Ctrl+Enter saves and closes; Ctrl+Shift+Enter saves and starts a new
//!   card in the same deck, with the focus in the front field. Both are
//!   caught in the capture phase, before the text views see the Enter.
//! - A blank front disables saving (the store's own rule).
//! - Closing with unsaved changes asks first.
//! - A store error keeps the dialog open with the text as typed.

use adw::prelude::*;
use adw::subclass::prelude::*;
use cards_core::domain::Card;
use gtk::glib;

use crate::state::AppState;
use crate::window::persistent_toast;

/// What a save did, for the deck page to update its list.
#[derive(Debug, Clone)]
pub enum Saved {
    Added(Card),
    Edited(Card),
}

type OnSaved = Box<dyn Fn(Saved)>;

mod imp {
    use std::cell::{OnceCell, RefCell};

    use adw::prelude::*;
    use adw::subclass::prelude::*;
    use gtk::glib;

    use super::OnSaved;
    use crate::state::AppState;

    #[derive(Default, gtk::CompositeTemplate)]
    #[template(resource = "/io/github/rsmirke/Cards/card_editor.ui")]
    pub struct CardEditor {
        #[template_child]
        pub toast_overlay: TemplateChild<adw::ToastOverlay>,
        #[template_child]
        pub front_view: TemplateChild<gtk::TextView>,
        #[template_child]
        pub back_view: TemplateChild<gtk::TextView>,
        pub state: OnceCell<AppState>,
        pub deck_id: OnceCell<String>,
        /// The card being edited; `None` for a new card.
        pub card_id: RefCell<Option<String>>,
        /// Front and back as last loaded or saved, to tell unsaved changes.
        pub saved_text: RefCell<(String, String)>,
        pub on_saved: OnceCell<OnSaved>,
    }

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
        // A new object's cells are empty, so these cannot fail.
        let _ = imp.state.set(state);
        let _ = imp.deck_id.set(deck_id.to_owned());
        let _ = imp.on_saved.set(Box::new(on_saved));
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
        let texts = self.texts();
        let can_save = can_save(&texts.0);
        self.action_set_enabled("editor.save", can_save);
        self.action_set_enabled("editor.save-and-new", can_save);
        // With unsaved changes, closing emits `close-attempt` instead.
        self.set_can_close(texts == *self.imp().saved_text.borrow());
    }

    fn save(&self, and_new: bool) {
        let imp = self.imp();
        let (front, back) = self.texts();
        if !can_save(&front) {
            return;
        }
        let (Some(state), Some(deck_id)) = (imp.state.get(), imp.deck_id.get()) else {
            return;
        };
        let card_id = imp.card_id.borrow().clone();
        // An unchanged card is not written (that would bump `updated_at`).
        if card_id.is_some() && (front.clone(), back.clone()) == *imp.saved_text.borrow() {
            if and_new {
                self.load(None, "", "");
                imp.front_view.grab_focus();
            } else {
                self.force_close();
            }
            return;
        }
        let result = match &card_id {
            Some(id) => state.edit_card(id, &front, &back).map(Saved::Edited),
            None => state.add_card(deck_id, &front, &back).map(Saved::Added),
        };
        let saved = match result {
            Ok(saved) => saved,
            Err(err) => {
                // Card text is user content: only at debug level (SPEC §2).
                tracing::warn!("a card could not be saved");
                tracing::debug!("{err}");
                imp.toast_overlay.add_toast(persistent_toast(&format!(
                    "The card could not be saved: {err}"
                )));
                return;
            }
        };
        let toast_text = match &saved {
            Saved::Added(_) => "Card added",
            Saved::Edited(_) => "Card saved",
        };
        if let Some(on_saved) = imp.on_saved.get() {
            on_saved(saved);
        }
        if and_new {
            self.load(None, "", "");
            imp.front_view.grab_focus();
            imp.toast_overlay.add_toast(
                adw::Toast::builder()
                    .title(toast_text)
                    .use_markup(false)
                    .build(),
            );
        } else {
            self.force_close();
        }
    }

    fn ask_discard(&self) {
        glib::spawn_future_local(glib::clone!(
            #[weak(rename_to = editor)]
            self,
            async move {
                let builder =
                    gtk::Builder::from_resource("/io/github/rsmirke/Cards/discard_dialog.ui");
                let Some(dialog) = builder.object::<adw::AlertDialog>("discard_dialog") else {
                    tracing::error!("discard_dialog.ui is missing its dialog");
                    return;
                };
                if dialog.choose_future(Some(&editor)).await == "discard" {
                    editor.force_close();
                }
            }
        ));
    }
}

/// Whether a card with this front can be saved: the same rule as the
/// store, which refuses a blank front.
fn can_save(front: &str) -> bool {
    !front.trim().is_empty()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_blank_front_cannot_be_saved() {
        assert!(!can_save(""));
        assert!(!can_save(" \n\t "));
        assert!(can_save(" x "));
    }
}
