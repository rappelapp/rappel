//! `CardObject`: one card as a list model item, for the deck page's
//! `GtkListView` (ADR 0013). It holds the card and its one-line preview,
//! computed once, so binding a recycled row is cheap.

use std::cell::{Ref, RefCell};

use cards_core::domain::Card;
use gtk::glib;
use gtk::subclass::prelude::*;

/// Longest preview, in characters. A label lays out all of its text even
/// when it ellipsises, so a huge front is cut before it gets there; more
/// than this never fits on one line anyway.
const PREVIEW_CHARS: usize = 200;

mod imp {
    use super::{Card, RefCell};
    use gtk::glib;
    use gtk::subclass::prelude::*;

    #[derive(Debug)]
    pub struct CardObject {
        pub card: RefCell<Card>,
        pub preview: RefCell<String>,
    }

    /// `GObject` needs a default; [`super::CardObject::new`], the only
    /// constructor, replaces this empty card at once.
    impl Default for CardObject {
        fn default() -> Self {
            Self {
                card: RefCell::new(Card {
                    id: String::new(),
                    deck_id: String::new(),
                    front_md: String::new(),
                    back_md: String::new(),
                    created_at: 0,
                    updated_at: 0,
                }),
                preview: RefCell::new(String::new()),
            }
        }
    }

    #[glib::object_subclass]
    impl ObjectSubclass for CardObject {
        const NAME: &'static str = "CardsCardObject";
        type Type = super::CardObject;
    }

    impl ObjectImpl for CardObject {}
}

glib::wrapper! {
    pub struct CardObject(ObjectSubclass<imp::CardObject>);
}

impl CardObject {
    pub fn new(card: Card) -> Self {
        let obj: Self = glib::Object::new();
        obj.imp().preview.replace(front_preview(&card.front_md));
        obj.imp().card.replace(card);
        obj
    }

    /// The card. An edit replaces the whole object in the model.
    pub fn card(&self) -> Ref<'_, Card> {
        self.imp().card.borrow()
    }

    /// The front on one line (see [`front_preview`]).
    pub fn preview(&self) -> Ref<'_, String> {
        self.imp().preview.borrow()
    }
}

/// `front` on one line: every run of whitespace (newlines included) becomes
/// one space, the ends are trimmed, and the result is cut to
/// [`PREVIEW_CHARS`] characters. The label ellipsises what does not fit.
pub fn front_preview(front: &str) -> String {
    let mut out = String::new();
    let mut chars = 0;
    for (i, word) in front.split_whitespace().enumerate() {
        if i > 0 {
            if chars + 1 >= PREVIEW_CHARS {
                break;
            }
            out.push(' ');
            chars += 1;
        }
        // Never copies more than the preview needs, even of a huge word.
        let room = PREVIEW_CHARS - chars;
        if let Some((cut, _)) = word.char_indices().nth(room) {
            out.push_str(&word[..cut]);
            break;
        }
        out.push_str(word);
        chars += word.chars().count();
        if chars >= PREVIEW_CHARS {
            break;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn whitespace_and_newlines_become_single_spaces() {
        assert_eq!(front_preview("  a\n\nb\t c  \r\n"), "a b c");
        assert_eq!(front_preview("one line"), "one line");
        assert_eq!(front_preview(""), "");
        assert_eq!(front_preview(" \n "), "");
    }

    #[test]
    fn markup_is_kept_as_text() {
        // The row's label has markup off; the preview does not escape.
        assert_eq!(front_preview("<b>x</b> &amp;"), "<b>x</b> &amp;");
    }

    #[test]
    fn long_fronts_are_cut_on_a_character_boundary() {
        let preview = front_preview(&"é".repeat(1_000));
        assert_eq!(preview.chars().count(), PREVIEW_CHARS);
        let preview = front_preview(&"漢字 ".repeat(1_000));
        assert_eq!(preview.chars().count(), PREVIEW_CHARS);
        assert!(preview.starts_with("漢字 漢字"));
        // A combining accent counts as its own character.
        let preview = front_preview(&"e\u{301}".repeat(150));
        assert_eq!(preview.chars().count(), PREVIEW_CHARS);
    }

    #[test]
    fn short_fronts_are_not_cut() {
        let front = "word ".repeat(39);
        assert_eq!(front_preview(&front), front.trim_end());
    }
}
