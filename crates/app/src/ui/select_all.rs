//! Ctrl+A for a page that lists cards: it chooses every card listed (#106).
//! All Cards and the deck page use it, through their shared selection mode
//! (`ui::selection`, ADR 0065). The page gives the action; this module decides when
//! the key is for it.
//!
//! **Why Capture, and what keeps the text fields' Ctrl+A.** GTK binds
//! Ctrl+A in several widgets on the way: `GtkText` selects all of its text
//! ("Shortcuts and Gestures" of the `GtkText` docs: "Ctrl+A or Ctrl+/ selects
//! all the text") and `GtkListView` binds `list.select-all` (`GtkListBase`
//! docs), which does nothing without a selection model, as in the card list.
//! A controller in the Capture phase on the page sees the key before all of
//! them (GTK's event propagation: capture from the root down to the target,
//! then bubble back up; `GtkPropagationPhase` docs), so it also sees a
//! Ctrl+A meant for a text field. Rather than depend on the text field
//! stopping it later, the controller looks at what has the focus and leaves
//! the key alone when that is an editable widget ([`selects_cards`]): the
//! search entry's `GtkText` (a `GtkEditable`), a `GtkTextView`. A key that
//! comes from a popover (a drop-down's popup has its own search field and
//! its own surface) is left alone too.

use gtk::gdk::{Key, ModifierType};
use gtk::glib;
use gtk::prelude::*;

/// The keys that choose every card: Ctrl+A, with Caps Lock or Shift
/// giving the capital. The Keyboard Shortcuts dialog lists them (a test
/// checks it).
pub const KEYS: [Key; 2] = [Key::a, Key::A];

/// Whether `key` with `modifiers` is Ctrl+A: Ctrl, and neither Alt nor
/// Super (Ctrl+Alt+A is another shortcut's).
fn is_key(key: Key, modifiers: ModifierType) -> bool {
    KEYS.contains(&key)
        && modifiers.contains(ModifierType::CONTROL_MASK)
        && !modifiers.intersects(ModifierType::ALT_MASK | ModifierType::SUPER_MASK)
}

/// Whether the key press chooses every card listed. `focus_in_text` is
/// whether the keyboard focus is in a field the user types in, whose own
/// Ctrl+A selects its text; `in_popover` whether the event comes from a
/// popover of the page.
pub fn selects_cards(
    key: Key,
    modifiers: ModifierType,
    focus_in_text: bool,
    in_popover: bool,
) -> bool {
    is_key(key, modifiers) && !focus_in_text && !in_popover
}

/// Whether `focus` is a field the user types in: a `GtkEditable` (an entry,
/// the `GtkText` inside a search entry) or a `GtkTextView`.
fn is_text_field(focus: Option<&gtk::Widget>) -> bool {
    focus.is_some_and(|w| w.is::<gtk::Editable>() || w.is::<gtk::TextView>())
}

/// Makes Ctrl+A on `page` call `on_select_all`, except where a text field
/// keeps it (see the module's text). The controller is in the Capture
/// phase and handles the key, so the list's own bindings never see it.
/// Returns the controller, named [`CONTROLLER_NAME`], for tests to find.
pub fn install(
    page: &impl IsA<gtk::Widget>,
    on_select_all: impl Fn() + 'static,
) -> gtk::EventControllerKey {
    let keys = gtk::EventControllerKey::new();
    keys.set_name(Some(CONTROLLER_NAME));
    keys.set_propagation_phase(gtk::PropagationPhase::Capture);
    let page_weak = page.downgrade();
    keys.connect_key_pressed(move |keys, key, _, modifiers| {
        let Some(page) = page_weak.upgrade() else {
            return glib::Propagation::Proceed;
        };
        // A popover has its own surface and its own focus (`GtkPopover` is
        // a `GtkNative`), so compare the event's surface with the page's.
        let in_popover = keys
            .current_event()
            .and_then(|event| event.surface())
            .is_some_and(|surface| Some(surface) != page.native().and_then(|n| n.surface()));
        let focus = page.root().and_then(|root| root.focus());
        if selects_cards(key, modifiers, is_text_field(focus.as_ref()), in_popover) {
            on_select_all();
            glib::Propagation::Stop
        } else {
            glib::Propagation::Proceed
        }
    });
    page.add_controller(keys.clone());
    keys
}

/// The name of the controller [`install`] makes.
pub const CONTROLLER_NAME: &str = "select-all";

#[cfg(test)]
mod tests {
    use super::*;

    const CTRL: ModifierType = ModifierType::CONTROL_MASK;

    #[test]
    fn ctrl_a_chooses_the_cards_where_no_text_field_has_the_focus() {
        assert!(selects_cards(Key::a, CTRL, false, false));
        assert!(selects_cards(
            Key::A,
            CTRL | ModifierType::SHIFT_MASK,
            false,
            false
        ));
        // Caps Lock does not matter.
        assert!(selects_cards(
            Key::a,
            CTRL | ModifierType::LOCK_MASK,
            false,
            false
        ));
    }

    #[test]
    fn a_text_field_keeps_its_own_ctrl_a() {
        assert!(!selects_cards(Key::a, CTRL, true, false));
        assert!(!selects_cards(Key::A, CTRL, true, false));
    }

    #[test]
    fn a_popover_keeps_its_keys() {
        assert!(!selects_cards(Key::a, CTRL, false, true));
    }

    #[test]
    fn other_keys_choose_nothing() {
        let none = ModifierType::empty();
        assert!(!selects_cards(Key::a, none, false, false), "a types");
        assert!(!selects_cards(Key::b, CTRL, false, false));
        assert!(!selects_cards(
            Key::a,
            CTRL | ModifierType::ALT_MASK,
            false,
            false
        ));
        assert!(!selects_cards(
            Key::a,
            ModifierType::SHIFT_MASK,
            false,
            false
        ));
    }

    #[test]
    fn only_a_field_the_user_types_in_is_a_text_field() {
        crate::testing::with_widgets(|| {
            let entry = gtk::Entry::new();
            let search = gtk::SearchEntry::new();
            let text = gtk::Text::new();
            let view = gtk::TextView::new();
            let button = gtk::Button::new();
            let list = gtk::ListView::new(None::<gtk::NoSelection>, None::<gtk::ListItemFactory>);
            for field in [
                entry.upcast_ref::<gtk::Widget>(),
                search.upcast_ref(),
                text.upcast_ref(),
                view.upcast_ref(),
            ] {
                assert!(is_text_field(Some(field)), "{field:?}");
            }
            for other in [button.upcast_ref::<gtk::Widget>(), list.upcast_ref()] {
                assert!(!is_text_field(Some(other)), "{other:?}");
            }
            assert!(!is_text_field(None));
        });
    }
}
