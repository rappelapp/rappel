//! Key hints in tooltips (#105, #117; ADR 0064): a button that a key also
//! presses says so in its tooltip, "Pass (2)", "Save (Ctrl+Enter)", instead
//! of a caption under it. The key's text is how GTK labels the trigger the
//! page binds (`gtk_accelerator_get_label`), in the user's language, so a
//! tooltip cannot name a key nothing binds.

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
}
