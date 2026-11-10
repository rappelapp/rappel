//! Key hints in tooltips (#105, #117; ADR 0064): a button that a key also
//! presses says so in its tooltip, "Pass (2)", "Save (Ctrl+Enter)", instead
//! of a caption under it. The key's text is how GTK labels the trigger the
//! page binds (`gtk_accelerator_get_label`), in the user's language, so a
//! tooltip cannot name a key nothing binds. Also the key that starts
//! studying (S, `starts_studying` and `StartHold`; ADR 0081).

use gtk::prelude::*;
use gtk::{gdk, glib};

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
/// Suspend), so they give `false`. Caps Lock (`LOCK_MASK`) is not a
/// modifier in this sense, and Caps Lock's capital S is still S; GDK 4 has
/// no Num Lock bit in `GdkModifierType` at all. The grades 1 to 4 are not
/// decided here: the review page binds them as shortcut triggers
/// (`pages::review_page::KEYS`).
pub fn starts_studying(
    key: gtk::gdk::Key,
    modifiers: gtk::gdk::ModifierType,
    text_focus: bool,
) -> bool {
    use gtk::gdk::ModifierType;
    let others = ModifierType::CONTROL_MASK
        | ModifierType::ALT_MASK
        | ModifierType::SHIFT_MASK
        | ModifierType::SUPER_MASK
        | ModifierType::META_MASK
        | ModifierType::HYPER_MASK;
    !text_focus && !modifiers.intersects(others) && is_s(key)
}

/// Whether `key` is the letter S, small or capital.
pub fn is_s(key: gtk::gdk::Key) -> bool {
    matches!(key, gtk::gdk::Key::s | gtk::gdk::Key::S)
}

/// S starts studying when it is *released*, not when it is pressed. A held
/// key repeats: GDK delivers each repeat as another key press with no
/// release between (`key-pressed` of `GtkEventControllerKey`,
/// gtkeventcontrollerkey.c; `key-released` comes once, when the key goes
/// up). Started on the press, the session's page would appear under the
/// repeats and its own S (Suspend) would suspend cards one after another.
/// So the press, and every repeat, is swallowed on the page that stays, and
/// the release starts the session once.
///
/// `T` is what S would start, recorded at the press: `()` on a deck's page,
/// the deck's id on the deck list, so a release on another row starts
/// nothing. A hold is only ever the *last* key press the page saw: every
/// other press (Ctrl, Shift, Return, …) cancels it, the release must be a
/// plain S too, and the page clears it when it is hidden, unmapped or loses
/// the focus ([`StartHold::clear`]). Otherwise an S pressed on one page and
/// released on another left a hold behind, and a later Ctrl+S started a
/// session.
#[derive(Debug)]
pub struct StartHold<T> {
    held: std::cell::RefCell<Option<T>>,
}

impl<T> Default for StartHold<T> {
    fn default() -> Self {
        Self {
            held: std::cell::RefCell::new(None),
        }
    }
}

impl<T: PartialEq> StartHold<T> {
    /// Every key press or repeat the page sees. `target` is what S would
    /// start now (`None`: nothing, as on a row that is not a deck's). Gives
    /// `true` when the page swallows the press: it is the S that starts
    /// studying, now held. Any other press cancels the hold.
    pub fn pressed(
        &self,
        key: gtk::gdk::Key,
        modifiers: gtk::gdk::ModifierType,
        text_focus: bool,
        target: Option<T>,
    ) -> bool {
        let held = target.filter(|_| starts_studying(key, modifiers, text_focus));
        let swallowed = held.is_some();
        self.held.replace(held);
        swallowed
    }

    /// A key release, with the modifiers it came with and what S would
    /// start now. `true` when it ends a held S as a plain S (no Ctrl, Alt,
    /// Shift, Super; no text field focused by now) on the same target as
    /// the press: the session starts. Only the first such release counts;
    /// the release of another key leaves the hold as it is (keys roll over
    /// when typing fast).
    pub fn released(
        &self,
        key: gtk::gdk::Key,
        modifiers: gtk::gdk::ModifierType,
        text_focus: bool,
        target: Option<&T>,
    ) -> bool {
        if !is_s(key) {
            return false;
        }
        let held = self.held.take();
        held.is_some() && held.as_ref() == target && starts_studying(key, modifiers, text_focus)
    }

    /// Every key press, seen in the capture phase before any child can
    /// claim it (Return on a row or a menu button never reaches the page's
    /// bubble phase): anything but a plain S cancels the hold.
    pub fn observe_press(&self, key: gtk::gdk::Key, modifiers: gtk::gdk::ModifierType) {
        if !starts_studying(key, modifiers, false) {
            self.clear();
        }
    }

    /// Forgets the hold: the page is hidden, unmapped or lost the focus,
    /// so the release may come anywhere.
    pub fn clear(&self) {
        self.held.replace(None);
    }
}

/// Calls `left` when `page` starts hiding (another page pushed over it, or
/// it is popped: `AdwNavigationPage::hiding`), is unmapped (GTK unmaps a
/// hidden page, adw-navigation-view.c sets its child-visible to false, and a
/// hidden or closed window unmaps its children), or the keyboard focus
/// leaves it (`GtkEventControllerFocus::leave`: into a dialog, which is not
/// a descendant of the page). A key held on `page` may then be released
/// anywhere, so a page forgets its held keys here.
pub fn when_left(
    page: &impl IsA<adw::NavigationPage>,
    left: impl Fn(&adw::NavigationPage) + 'static,
) {
    let left = std::rc::Rc::new(left);
    let page = page.upcast_ref::<adw::NavigationPage>();
    let on_hiding = std::rc::Rc::clone(&left);
    adw::prelude::NavigationPageExt::connect_hiding(page, move |page| on_hiding(page));
    let on_unmap = std::rc::Rc::clone(&left);
    page.connect_unmap(move |page| on_unmap(page));
    let focus = gtk::EventControllerFocus::new();
    let weak = page.downgrade();
    focus.connect_leave(move |_| {
        if let Some(page) = weak.upgrade() {
            left(&page);
        }
    });
    page.add_controller(focus);
}

/// What a page that starts studying with S does with keys
/// ([`install_start_key`]): its own [`StartHold`] and what S starts.
pub trait StartKeyPage: IsA<adw::NavigationPage> + IsA<gtk::Widget> {
    /// A key press or repeat the page saw in the bubble phase (a focused
    /// text field has had it first); Stop swallows it.
    fn start_key_pressed(&self, key: gdk::Key, modifiers: gdk::ModifierType) -> glib::Propagation;
    /// A key release the page saw.
    fn start_key_released(&self, key: gdk::Key, modifiers: gdk::ModifierType);
    /// Every key press, seen in the capture phase before any child can
    /// claim it (Return on a row or on a menu button never reaches the
    /// bubble phase): [`StartHold::observe_press`].
    fn observe_press(&self, key: gdk::Key, modifiers: gdk::ModifierType);
    /// Forgets a held S ([`StartHold::clear`]).
    fn clear_hold(&self);
}

/// Wires S to `page` (the deck list and a deck's page, ADR 0081): a
/// bubble-phase key controller for its presses and releases, a capture
/// phase one that sees every press first and returns Proceed, so it
/// claims neither the press nor its release (gtkeventcontrollerkey.c), and
/// [`when_left`], since a hold left behind would start a session at a
/// later release. A popover or a dialog cannot leak its S here: a
/// popover's native surface stops key events at its own root, and an
/// `AdwDialog` is not a descendant of the page.
pub fn install_start_key<P: StartKeyPage + glib::object::ObjectType>(page: &P) {
    let keys = gtk::EventControllerKey::new();
    let weak = page.downgrade();
    keys.connect_key_pressed(move |_, key, _, modifiers| {
        weak.upgrade().map_or(glib::Propagation::Proceed, |page| {
            page.start_key_pressed(key, modifiers)
        })
    });
    let weak = page.downgrade();
    keys.connect_key_released(move |_, key, _, modifiers| {
        if let Some(page) = weak.upgrade() {
            page.start_key_released(key, modifiers);
        }
    });
    page.add_controller(keys);
    let observer = gtk::EventControllerKey::new();
    observer.set_propagation_phase(gtk::PropagationPhase::Capture);
    let weak = page.downgrade();
    observer.connect_key_pressed(move |_, key, _, modifiers| {
        if let Some(page) = weak.upgrade() {
            page.observe_press(key, modifiers);
        }
        glib::Propagation::Proceed
    });
    page.add_controller(observer);
    when_left(page, |page| {
        if let Some(page) = page.downcast_ref::<P>() {
            page.clear_hold();
        }
    });
}

/// One action per press of a key, however long it is held: a held key
/// repeats as further presses with no release between (`key-pressed` and
/// `key-released` of `GtkEventControllerKey`, gtkeventcontrollerkey.c). The
/// review page's S suspends one card per press with this (ADR 0081).
///
/// The key is told by its hardware keycode, not its value: the press that
/// acted is matched to its release by the physical key, whatever the
/// layout makes of it. With a Cyrillic layout the key marked S gives
/// `Cyrillic_yeru`, which GTK's shortcut matching still takes for S
/// (`gdk_key_event_matches`, gdkevents.c: a partial match when the
/// keycode gives S in another layout that lacks the event's value), so a
/// keyval test on the release would never re-arm.
#[derive(Debug, Default)]
pub struct OnePress {
    /// The keycode of the press that acted, while that key is down.
    down: std::cell::Cell<Option<u32>>,
}

impl OnePress {
    /// A press or a repeat of the key with hardware keycode `code`: `true`
    /// for the first since the key that acted last was released (or ever,
    /// or since [`OnePress::reset`]).
    pub fn press(&self, code: u32) -> bool {
        if self.down.get().is_some() {
            return false;
        }
        self.down.set(Some(code));
        true
    }

    /// A key went up: when it is the key that acted, the next press acts
    /// again.
    pub fn release(&self, code: u32) {
        if self.down.get() == Some(code) {
            self.down.set(None);
        }
    }

    /// The release may have gone elsewhere (the page lost the focus): the
    /// next press acts.
    pub fn reset(&self) {
        self.down.set(None);
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
        // Caps Lock is not a modifier here (GDK 4 has no Num Lock bit).
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
        let hold = StartHold::<()>::default();
        // A release with no press seen starts nothing.
        assert!(!hold.released(Key::s, none, false, Some(&())));
        // The press and every repeat are swallowed; none starts.
        for _ in 0..5 {
            assert!(hold.pressed(Key::s, none, false, Some(())));
        }
        // Another key's release does not start, nor cancel; the first S
        // release does, once.
        assert!(!hold.released(Key::a, none, false, Some(&())));
        assert!(hold.released(Key::s, none, false, Some(&())));
        assert!(!hold.released(Key::s, none, false, Some(&())));
        // Other keys, modifiers, text fields and no target are not swallowed.
        assert!(!hold.pressed(Key::a, none, false, Some(())));
        assert!(!hold.pressed(Key::s, M::CONTROL_MASK, false, Some(())));
        assert!(!hold.pressed(Key::s, none, true, Some(())));
        assert!(!hold.pressed(Key::s, none, false, None));
        assert!(!hold.released(Key::s, none, false, Some(&())));
        // A text field that took the focus meanwhile cancels the start.
        assert!(hold.pressed(Key::S, none, false, Some(())));
        assert!(!hold.released(Key::S, none, true, Some(&())));
        assert!(!hold.released(Key::S, none, false, Some(&())));
        // So does a target gone meanwhile (selection mode began).
        assert!(hold.pressed(Key::s, none, false, Some(())));
        assert!(!hold.released(Key::s, none, false, None));
    }

    /// The stale hold of the review: any other press cancels it, so a later
    /// Ctrl+S or Shift+S cannot start a session.
    #[test]
    fn any_other_press_cancels_the_hold() {
        use gtk::gdk::{Key, ModifierType as M};
        let none = M::empty();
        for other in [Key::Control_L, Key::Shift_L, Key::Return, Key::a] {
            let hold = StartHold::<()>::default();
            assert!(hold.pressed(Key::s, none, false, Some(())));
            assert!(!hold.pressed(other, none, false, Some(())));
            assert!(!hold.released(Key::s, none, false, Some(&())), "{other:?}");
        }
    }

    #[test]
    fn a_release_with_a_modifier_starts_nothing() {
        use gtk::gdk::{Key, ModifierType as M};
        for modifiers in [M::CONTROL_MASK, M::SHIFT_MASK, M::ALT_MASK, M::SUPER_MASK] {
            let hold = StartHold::<()>::default();
            assert!(hold.pressed(Key::s, M::empty(), false, Some(())));
            assert!(
                !hold.released(Key::s, modifiers, false, Some(&())),
                "{modifiers:?}"
            );
            // The hold is spent: a plain release now starts nothing either.
            assert!(!hold.released(Key::s, M::empty(), false, Some(&())));
        }
        // Caps Lock is no modifier.
        let hold = StartHold::<()>::default();
        assert!(hold.pressed(Key::S, M::LOCK_MASK, false, Some(())));
        assert!(hold.released(Key::S, M::LOCK_MASK, false, Some(&())));
    }

    #[test]
    fn clearing_forgets_the_hold() {
        use gtk::gdk::{Key, ModifierType as M};
        let hold = StartHold::<()>::default();
        assert!(hold.pressed(Key::s, M::empty(), false, Some(())));
        // The page was hidden (another page pushed) or lost the focus.
        hold.clear();
        assert!(!hold.released(Key::s, M::empty(), false, Some(&())));
    }

    /// On the deck list the release must come on the row the press came on.
    #[test]
    fn the_release_must_be_on_the_target_of_the_press() {
        use gtk::gdk::{Key, ModifierType as M};
        let none = M::empty();
        let hold = StartHold::<&str>::default();
        assert!(hold.pressed(Key::s, none, false, Some("a")));
        assert!(!hold.released(Key::s, none, false, Some(&"b")));
        assert!(!hold.released(Key::s, none, false, Some(&"a")));
        assert!(hold.pressed(Key::s, none, false, Some("a")));
        assert!(!hold.released(Key::s, none, false, None));
        assert!(hold.pressed(Key::s, none, false, Some("a")));
        assert!(hold.released(Key::s, none, false, Some(&"a")));
    }

    #[test]
    fn one_press_acts_once_per_release() {
        const S: u32 = 39;
        const A: u32 = 38;
        let key = OnePress::default();
        // The first press ever acts; its repeats do not.
        assert!(key.press(S));
        assert!(!key.press(S));
        assert!(!key.press(S));
        // Another key's release does not re-arm.
        key.release(A);
        assert!(!key.press(S));
        key.release(S);
        assert!(key.press(S));
        assert!(!key.press(S));
        // A release seen elsewhere: reset lets the next press act.
        key.reset();
        assert!(key.press(S));
        // Releases without presses change nothing.
        key.release(S);
        key.release(S);
        assert!(key.press(S));
        assert!(!key.press(S));
    }

    #[test]
    fn a_press_seen_in_the_capture_phase_cancels_the_hold() {
        use gtk::gdk::{Key, ModifierType as M};
        let hold = StartHold::<()>::default();
        assert!(hold.pressed(Key::s, M::empty(), false, Some(())));
        // Its own repeats keep it.
        hold.observe_press(Key::s, M::empty());
        assert!(hold.released(Key::s, M::empty(), false, Some(&())));
        // Return on a menu button (the page's bubble phase never sees it).
        assert!(hold.pressed(Key::s, M::empty(), false, Some(())));
        hold.observe_press(Key::Return, M::empty());
        assert!(!hold.released(Key::s, M::empty(), false, Some(&())));
        // Ctrl+S is not a plain S either.
        assert!(hold.pressed(Key::s, M::empty(), false, Some(())));
        hold.observe_press(Key::s, M::CONTROL_MASK);
        assert!(!hold.released(Key::s, M::empty(), false, Some(&())));
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
