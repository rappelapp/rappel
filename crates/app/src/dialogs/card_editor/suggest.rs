//! The card editor's tags field: loading and checking it, and the
//! suggestions that appear under it while a name is typed. What the keys and
//! the text mean is decided by the pure functions in `tags.rs`.
//!
//! `GtkEntryCompletion` is deprecated since GTK 4.10, so the suggestions are
//! a [`TagSuggestions`] popover parented to the entry. The entry keeps the
//! keyboard focus throughout: the popover is not `autohide` (which GTK
//! documents as the setting that makes a popover modal, grabbing the
//! keyboard focus and closing on Esc), and its rows are not focusable. So
//! the entry's own key controller moves the highlight, accepts and closes
//! (Tab, Enter, Escape: `tags::key_action`).
//! That controller runs in the capture phase, which GTK delivers before the
//! bubble phase (`GtkPropagationPhase`), so Escape reaches it before the
//! dialog's own Escape handling: with suggestions showing, Escape closes only
//! them, and the dialog is left for the next Escape.

use adw::prelude::*;
use adw::subclass::prelude::*;
use gtk::{gdk, glib};

use super::tags::{self, KeyAction, Move, SuggestKey, TagsCheck};
use super::{CardEditor, TagSuggestions};
use crate::ui::toast::show_error;

impl CardEditor {
    /// Creates the suggestions popover and connects the entry's signals.
    pub(super) fn setup_tags(&self) {
        let imp = self.imp();
        let entry = &*imp.tags_entry;
        entry.update_property(&[gtk::accessible::Property::Autocomplete(
            gtk::AccessibleAutocomplete::List,
        )]);

        let popover = TagSuggestions::default();
        popover.set_parent(entry);
        popover.connect_chosen(glib::clone!(
            #[weak(rename_to = editor)]
            self,
            move |index| editor.accept_suggestion(index)
        ));
        if imp.suggestions.set(popover).is_err() {
            tracing::warn!("the tag suggestions were set up twice");
        }

        entry.connect_changed(glib::clone!(
            #[weak(rename_to = editor)]
            self,
            move |_| {
                editor.update_actions();
                // Not while the text is being filled in by the editor.
                if !editor.imp().filling.get() {
                    editor.imp().edited.set(true);
                    editor.update_suggestions();
                }
            }
        ));
        entry.connect_notify_local(
            Some("cursor-position"),
            glib::clone!(
                #[weak(rename_to = editor)]
                self,
                move |_, _| {
                    let imp = editor.imp();
                    // The cursor moves after every typed character: that
                    // is not a move. A move by itself (arrows, a click)
                    // closes the list, so a later Tab cannot rewrite a name
                    // the user did not touch.
                    if !imp.filling.get() && !imp.edited.replace(false) {
                        editor.close_suggestions();
                    }
                }
            ),
        );

        let keys = gtk::EventControllerKey::new();
        keys.set_propagation_phase(gtk::PropagationPhase::Capture);
        keys.connect_key_pressed(glib::clone!(
            #[weak(rename_to = editor)]
            self,
            #[upgrade_or]
            glib::Propagation::Proceed,
            move |_, key, _, state| editor.tags_key(key, state)
        ));
        entry.add_controller(keys);

        // Tab or a click elsewhere: the suggestions go with the focus.
        let focus = gtk::EventControllerFocus::new();
        focus.connect_leave(glib::clone!(
            #[weak(rename_to = editor)]
            self,
            move |_| editor.close_suggestions()
        ));
        entry.add_controller(focus);
    }

    /// Reads every tag again, for the suggestions. A failure only means no
    /// suggestions.
    pub(super) fn reload_all_tags(&self) {
        let imp = self.imp();
        let Some(state) = imp.state.get() else {
            return;
        };
        match state.tags() {
            Ok(all) => {
                imp.all_tags.replace(all);
            }
            Err(err) => {
                tracing::warn!("the tags could not be read for suggestions: {err}");
                imp.all_tags.replace(Vec::new());
            }
        }
    }

    /// The text of the tags field for card `card_id`. If its tags cannot be
    /// read the field is disabled, an error is shown and a save leaves the
    /// card's tags alone.
    pub(super) fn read_card_tags(&self, card_id: &str) -> String {
        let imp = self.imp();
        let Some(state) = imp.state.get() else {
            return String::new();
        };
        match state.card_tags(card_id) {
            Ok(card_tags) => {
                imp.tags_loaded.set(true);
                imp.tags_entry.set_sensitive(true);
                tags::field_text(&card_tags)
            }
            Err(err) => {
                imp.tags_loaded.set(false);
                imp.tags_entry.set_sensitive(false);
                let message = format!("The card's tags could not be read: {err}");
                imp.tags_error
                    .set_label("The tags could not be read, so they are left as they are.");
                imp.tags_error.set_visible(true);
                show_error(&*imp.toast_overlay, "reading a card's tags", &message);
                String::new()
            }
        }
    }

    /// The field's names, or what is wrong with them. A disabled field
    /// (tags not loaded) has nothing to check.
    pub(super) fn tags_check(&self) -> TagsCheck {
        let imp = self.imp();
        if imp.tags_loaded.get() {
            tags::check_tags(&imp.tags_entry.text())
        } else {
            TagsCheck {
                names: Vec::new(),
                error: None,
            }
        }
    }

    /// Lists the tags that match the name being typed, under the entry.
    fn update_suggestions(&self) {
        let imp = self.imp();
        let (Some(popover), true) = (imp.suggestions.get(), imp.tags_loaded.get()) else {
            return;
        };
        let entry = &*imp.tags_entry;
        // Only while the user is typing in it, not when a card is loaded.
        // The focus is on the entry's inner `GtkText`, not on the entry.
        let typing = self.focus().is_some_and(|focus| {
            focus == *entry.upcast_ref::<gtk::Widget>() || focus.is_ancestor(entry)
        });
        if !typing {
            self.close_suggestions();
            return;
        }
        let cursor = usize::try_from(entry.position()).unwrap_or(0);
        let found = tags::suggestions(&imp.all_tags.borrow(), &entry.text(), cursor);
        if found.is_empty() {
            self.close_suggestions();
            return;
        }
        let same = found
            .iter()
            .map(|t| &t.id)
            .eq(imp.suggested.borrow().iter().map(|t| &t.id));
        if !same || !popover.is_visible() {
            let names: Vec<&str> = found.iter().map(|t| t.name.as_str()).collect();
            let count = names.len();
            popover.set_names(&names);
            popover.set_width_request(entry.width());
            // Nothing is highlighted until Down or Up: Tab takes the
            // first, Enter keeps the typed text.
            imp.highlighted.set(None);
            popover.highlight(None);
            let opening = !popover.is_visible();
            imp.suggested.replace(found);
            if opening {
                popover.popup();
                entry.announce(
                    &tags::suggestions_announcement(count),
                    gtk::AccessibleAnnouncementPriority::Low,
                );
            }
        }
    }

    pub(super) fn close_suggestions(&self) {
        let imp = self.imp();
        if let Some(popover) = imp.suggestions.get()
            && popover.is_visible()
        {
            popover.popdown();
        }
        imp.suggested.borrow_mut().clear();
        imp.highlighted.set(None);
    }

    /// The tags entry's keys while suggestions show, as `tags::key_action`
    /// decides: Down / Up highlight, Tab takes the highlighted (else the
    /// first) suggestion, Enter takes a highlighted one only, Escape closes
    /// the list. Anything else (and every key while no suggestion shows) is
    /// the entry's own.
    fn tags_key(&self, key: gdk::Key, state: gdk::ModifierType) -> glib::Propagation {
        let imp = self.imp();
        let showing = imp
            .suggestions
            .get()
            .is_some_and(gtk::prelude::WidgetExt::is_visible);
        // Ctrl+Enter and friends are the editor's save keys.
        let plain = !state.intersects(
            gdk::ModifierType::CONTROL_MASK
                | gdk::ModifierType::ALT_MASK
                | gdk::ModifierType::SHIFT_MASK,
        );
        if !showing || !plain {
            return glib::Propagation::Proceed;
        }
        let key = match key {
            gdk::Key::Down | gdk::Key::KP_Down => SuggestKey::Down,
            gdk::Key::Up | gdk::Key::KP_Up => SuggestKey::Up,
            gdk::Key::Return | gdk::Key::KP_Enter | gdk::Key::ISO_Enter => SuggestKey::Enter,
            gdk::Key::Tab => SuggestKey::Tab,
            gdk::Key::Escape => SuggestKey::Escape,
            _ => return glib::Propagation::Proceed,
        };
        match tags::key_action(key, true, imp.highlighted.get()) {
            KeyAction::Highlight(movement) => self.move_highlight(movement),
            KeyAction::Accept(index) => self.accept_suggestion(index),
            KeyAction::Close => self.close_suggestions(),
            KeyAction::KeepTyped => {
                self.close_suggestions();
                return glib::Propagation::Proceed;
            }
            KeyAction::Pass => return glib::Propagation::Proceed,
        }
        glib::Propagation::Stop
    }

    fn move_highlight(&self, movement: Move) {
        let imp = self.imp();
        let suggested = imp.suggested.borrow();
        let Some(index) = tags::highlight_after(movement, imp.highlighted.get(), suggested.len())
        else {
            return;
        };
        imp.highlighted.set(Some(index));
        if let Some(popover) = imp.suggestions.get() {
            popover.highlight(Some(index));
        }
        if let Some(tag) = suggested.get(index) {
            imp.tags_entry.announce(
                &tags::highlight_announcement(&tag.name, index, suggested.len()),
                gtk::AccessibleAnnouncementPriority::Medium,
            );
        }
    }

    /// Puts suggestion `index` in place of the name being typed.
    pub(super) fn accept_suggestion(&self, index: usize) {
        let imp = self.imp();
        let Some(tag) = imp.suggested.borrow().get(index).cloned() else {
            return;
        };
        let entry = &*imp.tags_entry;
        let text = entry.text();
        let cursor = usize::try_from(entry.position()).unwrap_or(0);
        let typed = tags::name_at(&text, cursor);
        let (new_text, new_cursor) = tags::accept(&text, typed, &tag.name);
        self.close_suggestions();
        // `set_text` replaces everything with the cursor at the start: the
        // suggestions must not follow it.
        imp.filling.set(true);
        entry.set_text(&new_text);
        entry.set_position(i32::try_from(new_cursor).unwrap_or(-1));
        imp.filling.set(false);
    }
}
