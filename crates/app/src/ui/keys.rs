//! Key hints in tooltips (#105, #117; ADR 0064): a button that a key also
//! presses says so in its tooltip, "Pass (2)", "Save (Ctrl+Enter)", instead
//! of a caption under it. The key's text is how GTK labels the trigger the
//! page binds (`gtk_accelerator_get_label`), in the user's language, so a
//! tooltip cannot name a key nothing binds. Also the key that starts
//! studying (S, `starts_studying` and `StartHold`; ADR 0081).

use gtk::prelude::*;

use crate::i18n::{self, gettext, pgettext};

/// The tooltip of a button that a key also presses: "Pass (2)". `name` is
/// the button's name without its mnemonic, `key` the key as the user sees
/// it ("Space", "Ctrl+Enter").
pub fn tooltip(name: &str, key: &str) -> String {
    // Translators: the tooltip of a button that a key also presses.
    // {name} is the button ("Pass"), {key} the key ("2").
    i18n::fill(&gettext("{name} ({key})"), &[("name", name), ("key", key)])
}

/// How GTK labels `trigger` ("space", "<Control>Return": the strings of
/// `GtkShortcutTrigger`) the way it shows an accelerator in a menu:
/// "Space", "Ctrl+Enter". `None` for a trigger GTK cannot parse. GTK names
/// the Return key "Return", which no keyboard says: it is "Enter" here, in
/// the user's language.
pub fn label(trigger: &str) -> Option<String> {
    let (keyval, modifiers) = gtk::accelerator_parse(trigger)?;
    let label = gtk::accelerator_get_label(keyval, modifiers).to_string();
    if keyval == gtk::gdk::Key::Return {
        let key = gtk::accelerator_get_label(keyval, gtk::gdk::ModifierType::empty());
        if let Some(modifiers) = label.strip_suffix(key.as_str()) {
            // Translators: the Return key as a tooltip names it, after the
            // modifiers: "Ctrl+Enter".
            return Some(format!("{modifiers}{}", pgettext("key name", "Enter")));
        }
    }
    Some(label)
}

/// The trigger the deck pages bind to start studying, for the tooltip.
pub const START_STUDY_TRIGGER: &str = "s";

/// Whether `key` with `modifiers` is the plain S that starts studying,
/// given whether a text field has the keyboard focus (`text_focus`: a typed
/// letter belongs to it). Ctrl, Alt, Shift, Super, Meta and Hyper make it
/// another shortcut (Ctrl+S, Shift+S, which the review page binds to
/// Suspend), so they give `false`. Caps Lock and Num Lock are not
/// modifiers in this sense, and Caps Lock's capital S is still S. The
/// grades 1 to 4 are not decided here: the review page binds them as
/// shortcut triggers (`pages::review_page::KEYS`).
pub fn starts_studying(
    key: gtk::gdk::Key,
    modifiers: gtk::gdk::ModifierType,
    text_focus: bool,
) -> bool {
    use gtk::gdk::{Key, ModifierType};
    let others = ModifierType::CONTROL_MASK
        | ModifierType::ALT_MASK
        | ModifierType::SHIFT_MASK
        | ModifierType::SUPER_MASK
        | ModifierType::META_MASK
        | ModifierType::HYPER_MASK;
    !text_focus && !modifiers.intersects(others) && matches!(key, Key::s | Key::S)
}

/// S starts studying when it is *released*, not when it is pressed. A held
/// key repeats: GDK delivers each repeat as another key press with no
/// release between (`key-pressed` of `GtkEventControllerKey`,
/// gtkeventcontrollerkey.c; `key-released` comes once, when the key goes
/// up). Started on the press, the session's page would appear under the
/// repeats and its own S (Suspend) would suspend cards one after another.
/// So the press, and every repeat, is swallowed on the page that stays, and
/// the release starts the session once.
#[derive(Debug, Default)]
pub struct StartHold {
    held: std::cell::Cell<bool>,
}

impl StartHold {
    /// A key press or repeat: `true` when the page swallows it (it is the
    /// S that starts studying, now held).
    pub fn pressed(
        &self,
        key: gtk::gdk::Key,
        modifiers: gtk::gdk::ModifierType,
        text_focus: bool,
    ) -> bool {
        let start = starts_studying(key, modifiers, text_focus);
        if start {
            self.held.set(true);
        }
        start
    }

    /// A key release: `true` when it ends a held S and no text field has
    /// the focus now, so the session starts. Only the first release counts.
    pub fn released(&self, key: gtk::gdk::Key, text_focus: bool) -> bool {
        let is_s = matches!(key, gtk::gdk::Key::s | gtk::gdk::Key::S);
        is_s && self.held.take() && !text_focus
    }
}

/// Whether the keyboard focus is in a text field (an entry, a search
/// field, a text view) under `root`: a key typed there is text.
pub fn text_has_focus(root: &impl IsA<gtk::Widget>) -> bool {
    let Some(focus) = root.root().and_then(|r| gtk::prelude::RootExt::focus(&r)) else {
        return false;
    };
    // A GtkText inside a GtkEntry or GtkSearchEntry is the focus widget;
    // GtkTextView is an editable-like widget of its own.
    // `gtk_widget_get_ancestor` counts the widget itself.
    [gtk::Editable::static_type(), gtk::TextView::static_type()]
        .into_iter()
        .any(|kind| focus.ancestor(kind).is_some())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_tooltip_names_the_button_and_its_key() {
        assert_eq!(tooltip("Pass", "2"), "Pass (2)");
        assert_eq!(tooltip("Fail", "1"), "Fail (1)");
        assert_eq!(tooltip("Show Answer", "Space"), "Show Answer (Space)");
        assert_eq!(tooltip("Save", "Ctrl+Enter"), "Save (Ctrl+Enter)");
        // A name or a key with braces is not read as a placeholder.
        assert_eq!(tooltip("{key}", "{name}"), "{key} ({name})");
    }

    #[test]
    fn enter_is_named_in_the_users_language() {
        crate::testing::with_widgets(|| {
            crate::i18n::tests::in_french(|| {
                assert_eq!(label("<Control>Return").as_deref(), Some("Ctrl+Entrée"));
                assert_eq!(label("Return").as_deref(), Some("Entrée"));
            });
        });
    }

    #[test]
    fn a_trigger_is_labelled_as_a_menu_would() {
        crate::testing::with_widgets(|| {
            assert_eq!(label("space").as_deref(), Some("Space"));
            assert_eq!(label("2").as_deref(), Some("2"));
            assert_eq!(label("<Control>Return").as_deref(), Some("Ctrl+Enter"));
            assert_eq!(label("Return").as_deref(), Some("Enter"));
            assert_eq!(label("<Control>z").as_deref(), Some("Ctrl+Z"));
            assert_eq!(label("e").as_deref(), Some("E"));
            assert_eq!(label("not a key"), None);
        });
    }

    #[test]
    fn s_starts_studying_only_as_a_plain_key_outside_text_fields() {
        use gtk::gdk::{Key, ModifierType as M};
        let none = M::empty();
        assert!(starts_studying(Key::s, none, false));
        // Caps Lock (and Num Lock) are not modifiers here.
        assert!(starts_studying(Key::S, M::LOCK_MASK, false));
        assert!(starts_studying(Key::s, M::LOCK_MASK, false));
        assert!(!starts_studying(Key::s, none, true));
        for m in [
            M::CONTROL_MASK,
            M::ALT_MASK,
            M::SHIFT_MASK,
            M::SUPER_MASK,
            M::META_MASK,
            M::HYPER_MASK,
            M::CONTROL_MASK | M::SHIFT_MASK,
        ] {
            assert!(!starts_studying(Key::s, m, false), "{m:?}");
            assert!(!starts_studying(Key::S, m, false), "{m:?}");
        }
        // Unrelated keys, the grades among them.
        for k in [
            Key::_1,
            Key::_2,
            Key::_3,
            Key::KP_4,
            Key::_0,
            Key::a,
            Key::e,
            Key::space,
            Key::Return,
            Key::Escape,
        ] {
            assert!(!starts_studying(k, none, false), "{k:?}");
        }
    }

    #[test]
    fn a_held_s_starts_once_on_its_release() {
        use gtk::gdk::{Key, ModifierType as M};
        let none = M::empty();
        let hold = StartHold::default();
        // A release with no press seen starts nothing.
        assert!(!hold.released(Key::s, false));
        // The press and every repeat are swallowed; none starts.
        for _ in 0..5 {
            assert!(hold.pressed(Key::s, none, false));
        }
        // Another key's release does not start; the first S release does,
        // once.
        assert!(!hold.released(Key::a, false));
        assert!(hold.released(Key::s, false));
        assert!(!hold.released(Key::s, false));
        // Other keys, modifiers and text fields are not swallowed.
        assert!(!hold.pressed(Key::a, none, false));
        assert!(!hold.pressed(Key::s, M::CONTROL_MASK, false));
        assert!(!hold.pressed(Key::s, none, true));
        assert!(!hold.released(Key::s, false));
        // A text field that took the focus meanwhile cancels the start.
        assert!(hold.pressed(Key::S, none, false));
        assert!(!hold.released(Key::S, true));
        assert!(!hold.released(Key::S, false));
    }

    #[test]
    fn a_text_field_in_a_window_has_the_focus() {
        crate::testing::with_widgets(|| {
            let entry = gtk::SearchEntry::new();
            let button = gtk::Button::new();
            let row = gtk::Box::new(gtk::Orientation::Horizontal, 0);
            row.append(&entry);
            row.append(&button);
            let window = gtk::Window::new();
            window.set_child(Some(&row));
            window.present();
            button.grab_focus();
            assert!(!text_has_focus(&button));
            entry.grab_focus();
            assert!(text_has_focus(&button));
            assert!(text_has_focus(&row));
            // Not in a window: no focus at all.
            assert!(!text_has_focus(&gtk::Button::new()));
            window.close();
        });
    }
}
