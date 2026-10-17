//! Tests of the Keyboard Shortcuts dialog (`data/shortcuts-dialog.blp`).
//! It has no code: `AdwApplication` loads it and sets up `app.shortcuts`
//! and Ctrl+? itself. These tests keep it honest: every accelerator in it
//! parses, and every key the code binds is listed.

use adw::prelude::*;
use gtk::gio;

use crate::dialogs::card_editor::{FORMAT_KEYS, SAVE_KEYS};
use crate::ui::dialog;

const FILE: &str = "shortcuts-dialog.ui";

/// A key and its modifiers, the way GTK compares accelerators.
type Accel = (gtk::gdk::Key, gtk::gdk::ModifierType);

/// `trigger` as an accelerator, ignoring Shift on a letter (the review
/// page binds `e` and `<Shift>e` alike) and the keypad (`KP_1` is `1`).
fn normalised(trigger: &str) -> Accel {
    let (key, mods) = gtk::accelerator_parse(trigger)
        .unwrap_or_else(|| panic!("{trigger:?} is not an accelerator"));
    let key = match key {
        gtk::gdk::Key::KP_Enter => gtk::gdk::Key::Return,
        gtk::gdk::Key::KP_Delete => gtk::gdk::Key::Delete,
        gtk::gdk::Key::KP_1 => gtk::gdk::Key::_1,
        gtk::gdk::Key::KP_2 => gtk::gdk::Key::_2,
        other => other.to_lower(),
    };
    let mods = if key.to_unicode().is_some_and(char::is_alphabetic)
        && !mods.contains(gtk::gdk::ModifierType::CONTROL_MASK)
    {
        mods - gtk::gdk::ModifierType::SHIFT_MASK
    } else {
        mods
    };
    (key, mods)
}

/// Every item of the dialog as (title, accelerators).
fn items() -> Vec<(String, Vec<String>)> {
    let builder = dialog::builder(FILE);
    let _: adw::ShortcutsDialog = dialog::object(&builder, FILE, "shortcuts_dialog")
        .expect("AdwApplication needs the id shortcuts_dialog");
    builder
        .objects()
        .into_iter()
        .filter_map(|o| o.downcast::<adw::ShortcutsItem>().ok())
        .map(|item| {
            let accels = item
                .accelerator()
                .split_whitespace()
                .map(str::to_owned)
                .collect();
            (item.title().to_string(), accels)
        })
        .collect()
}

#[test]
fn every_accelerator_parses_and_every_bound_key_is_listed() {
    crate::testing::with_gtk(|| {
        adw::init().unwrap();
        gio::resources_register_include!("rappel.gresource").unwrap();
        let items = items();
        assert!(items.len() >= 20, "the dialog lost its items");
        let mut listed = Vec::new();
        for (title, accels) in &items {
            assert!(!accels.is_empty(), "{title} has no accelerator");
            for accel in accels {
                listed.push(normalised(accel));
            }
        }
        let bound = crate::ACCELS
            .iter()
            .map(|(_, accel)| *accel)
            .chain(["F10", "<Control>question"])
            .chain(crate::pages::review_key_triggers())
            .chain(SAVE_KEYS.iter().map(|(trigger, _)| *trigger))
            .chain(FORMAT_KEYS.iter().map(|(trigger, _)| *trigger));
        for trigger in bound {
            assert!(
                listed.contains(&normalised(trigger)),
                "{trigger} is bound but not in the Keyboard Shortcuts dialog"
            );
        }
    });
}
