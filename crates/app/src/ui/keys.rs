//! Key hints in tooltips (#105, #117; ADR 0064): a button that a key also
//! presses says so in its tooltip, "Pass (2)", "Save (Ctrl+Enter)", instead
//! of a caption under it. The key's text is how GTK labels the trigger the
//! page binds (`gtk_accelerator_get_label`), in the user's language, so a
//! tooltip cannot name a key nothing binds.

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

/// What a study key asks for (ADR 0081).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StudyKey {
    /// S on a deck's page or on a row of the deck list: start studying.
    Start,
    /// 1: Fail.
    Fail,
    /// 2, 3 or 4: Pass (Anki's Good and Easy also mean Pass here).
    Pass,
}

/// The trigger the deck pages bind to start studying, for the tooltip.
pub const START_STUDY_TRIGGER: &str = "s";

/// The study key that `key` with `modifiers` is, given whether a text field
/// has the keyboard focus (`text_focus`: a typed letter or digit belongs to
/// it). Keypad digits count as the main row's. Ctrl, Alt, Shift and Super
/// make a key something else (Ctrl+S, Shift+2 = "@"), so they give `None`;
/// Caps Lock and Num Lock are not modifiers in this sense, and Caps Lock's
/// capital S is still S.
pub fn study_key(
    key: gtk::gdk::Key,
    modifiers: gtk::gdk::ModifierType,
    text_focus: bool,
) -> Option<StudyKey> {
    use gtk::gdk::{Key, ModifierType};
    let others = ModifierType::CONTROL_MASK
        | ModifierType::ALT_MASK
        | ModifierType::SHIFT_MASK
        | ModifierType::SUPER_MASK
        | ModifierType::META_MASK;
    if text_focus || modifiers.intersects(others) {
        return None;
    }
    match key {
        Key::s | Key::S => Some(StudyKey::Start),
        Key::_1 | Key::KP_1 => Some(StudyKey::Fail),
        Key::_2 | Key::_3 | Key::_4 | Key::KP_2 | Key::KP_3 | Key::KP_4 => Some(StudyKey::Pass),
        _ => None,
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
    fn study_keys_are_plain_keys_outside_text_fields() {
        use gtk::gdk::{Key, ModifierType as M};
        let none = M::empty();
        let key = |k, m, t| study_key(k, m, t);
        assert_eq!(key(Key::s, none, false), Some(StudyKey::Start));
        assert_eq!(key(Key::S, M::LOCK_MASK, false), Some(StudyKey::Start));
        assert_eq!(key(Key::_1, none, false), Some(StudyKey::Fail));
        assert_eq!(key(Key::KP_1, M::LOCK_MASK, false), Some(StudyKey::Fail));
        for k in [Key::_2, Key::_3, Key::_4, Key::KP_2, Key::KP_3, Key::KP_4] {
            assert_eq!(key(k, none, false), Some(StudyKey::Pass), "{k:?}");
            // Num Lock (Mod2) does not change a keypad digit.
            assert_eq!(key(k, M::LOCK_MASK, false), Some(StudyKey::Pass));
            assert_eq!(key(k, none, true), None, "text field: {k:?}");
        }
        assert_eq!(key(Key::s, none, true), None);
        for m in [
            M::CONTROL_MASK,
            M::ALT_MASK,
            M::SHIFT_MASK,
            M::SUPER_MASK,
            M::META_MASK,
            M::CONTROL_MASK | M::SHIFT_MASK,
        ] {
            for k in [
                Key::s,
                Key::S,
                Key::_1,
                Key::_2,
                Key::_3,
                Key::_4,
                Key::KP_3,
            ] {
                assert_eq!(key(k, m, false), None, "{k:?} with {m:?}");
            }
        }
        // Unrelated keys, including the digits next to the grades.
        for k in [
            Key::_0,
            Key::_5,
            Key::KP_5,
            Key::a,
            Key::e,
            Key::space,
            Key::Return,
            Key::Escape,
        ] {
            assert_eq!(key(k, none, false), None, "{k:?}");
        }
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
