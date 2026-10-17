//! `CardObject`: one card as a list model item, for the deck page's
//! `GtkListView` (ADR 0013). It holds the card and its one-line preview
//! (plain text, never markup), computed once, so binding a recycled row is
//! cheap. It also holds the card's tags as one line, like the preview.

use std::cell::{Ref, RefCell};

use gtk::glib;
use gtk::subclass::prelude::*;
use rappel_core::content::plain_summary;
use rappel_core::domain::{Card, Tag, TaggedCard};

mod imp {
    use super::{Card, RefCell};
    use gtk::glib;
    use gtk::subclass::prelude::*;

    #[derive(Debug)]
    pub struct CardObject {
        pub card: RefCell<Card>,
        pub preview: RefCell<String>,
        pub tags_line: RefCell<String>,
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
                    suspended: false,
                }),
                preview: RefCell::new(String::new()),
                tags_line: RefCell::new(String::new()),
            }
        }
    }

    #[glib::object_subclass]
    impl ObjectSubclass for CardObject {
        const NAME: &'static str = "RappelCardObject";
        type Type = super::CardObject;
    }

    impl ObjectImpl for CardObject {}
}

glib::wrapper! {
    pub struct CardObject(ObjectSubclass<imp::CardObject>);
}

impl CardObject {
    pub fn new(tagged: TaggedCard) -> Self {
        let TaggedCard { card, tags } = tagged;
        let obj: Self = glib::Object::new();
        obj.imp().preview.replace(plain_summary(&card.front_md));
        obj.imp().tags_line.replace(tags_line(&tags));
        obj.imp().card.replace(card);
        obj
    }

    /// The card. An edit replaces the whole object in the model.
    pub fn card(&self) -> Ref<'_, Card> {
        self.imp().card.borrow()
    }

    /// The front on one line, without Markdown syntax (see
    /// [`plain_summary`]).
    pub fn preview(&self) -> Ref<'_, String> {
        self.imp().preview.borrow()
    }

    /// The tags on one line (see [`tags_line`]), empty without tags.
    pub fn tags_line(&self) -> Ref<'_, String> {
        self.imp().tags_line.borrow()
    }

    /// What a screen reader says for the row (see [`row_label`]).
    pub fn row_label(&self) -> String {
        row_label(&self.preview(), &self.tags_line())
    }
}

/// The tag names joined by ", ", each on one line (tag names are only
/// trimmed, so a pasted one can hold a line break or runs of spaces).
/// Plain text, never markup.
pub fn tags_line(tags: &[Tag]) -> String {
    tags.iter()
        .map(|tag| tag.name.split_whitespace().collect::<Vec<_>>().join(" "))
        .collect::<Vec<_>>()
        .join(", ")
}

/// The accessible label of a row: the front, then "tags:" and the tags
/// line when there are tags.
pub fn row_label(preview: &str, tags_line: &str) -> String {
    if tags_line.is_empty() {
        preview.to_owned()
    } else {
        format!("{preview}, tags: {tags_line}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tag(name: &str) -> Tag {
        Tag {
            id: format!("id-{name}"),
            name: name.to_owned(),
        }
    }

    #[test]
    fn tags_are_one_plain_line() {
        assert_eq!(tags_line(&[]), "");
        assert_eq!(tags_line(&[tag("maths")]), "maths");
        assert_eq!(
            tags_line(&[tag("maths"), tag("a\n  b"), tag("<b>x</b>")]),
            "maths, a b, <b>x</b>"
        );
    }

    #[test]
    fn a_row_is_read_with_its_tags() {
        assert_eq!(row_label("2 + 2", ""), "2 + 2");
        assert_eq!(
            row_label("2 + 2", "maths, easy"),
            "2 + 2, tags: maths, easy"
        );
    }
}
