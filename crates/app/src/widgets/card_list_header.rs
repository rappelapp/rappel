//! `CardListHeader`: the line above a list of cards with the tags every
//! listed card has (ADR 0077), on the deck page and All Cards. Layout in
//! `data/card_list_header.blp`. It sits above the list's scrolled window,
//! not inside the list as a `GtkListView` section header: a list view
//! scrolls its focused row to the top of the view when the row is focused
//! before it has been laid out (`gtk_list_base_scroll_to_item`, which
//! anchors an item not yet allocated at the top, gtk/gtklistbase.c), and
//! opening a page focuses its first row, so a header above that row was
//! scrolled out of sight every time (seen under Xvfb: the view scrolled
//! by the header's 42 px). Hidden while no tag is shared. It never
//! scrolls, so it names at most [`HEADER_TAGS`] tags and one "+N" pill,
//! whose tooltip names the others: a deck whose cards share dozens of tags
//! (an import) still leaves the list room at 360 × 640.

use gtk::glib;

/// The most tags the header shows as pills before its "+N" pill.
pub const HEADER_TAGS: u32 = 6;

mod imp {
    use std::cell::RefCell;

    use adw::prelude::*;
    use adw::subclass::prelude::*;
    use gtk::glib;

    use crate::widgets::TagPills;

    #[derive(Debug, Default, gtk::CompositeTemplate, glib::Properties)]
    #[properties(wrapper_type = super::CardListHeader)]
    #[template(resource = "/io/github/rappelapp/Rappel/card_list_header.ui")]
    pub struct CardListHeader {
        #[template_child]
        pub shared_label: TemplateChild<gtk::Label>,
        #[template_child]
        pub shared_pills: TemplateChild<TagPills>,
        /// The "+N" pill after the pills, past [`super::HEADER_TAGS`].
        pub overflow: gtk::Label,
        /// The names of the tags every listed card has, one line each.
        #[property(get, set = Self::set_tags)]
        pub tags: RefCell<Vec<String>>,
    }

    impl CardListHeader {
        fn set_tags(&self, tags: Vec<String>) {
            super::show_capped(&self.shared_pills, &self.overflow, &tags);
            self.obj().set_visible(!tags.is_empty());
            self.tags.replace(tags);
        }
    }

    #[glib::object_subclass]
    impl ObjectSubclass for CardListHeader {
        const NAME: &'static str = "RappelCardListHeader";
        type Type = super::CardListHeader;
        type ParentType = gtk::Box;

        fn class_init(klass: &mut Self::Class) {
            TagPills::ensure_type();
            klass.bind_template();
        }

        fn instance_init(obj: &glib::subclass::InitializingObject<Self>) {
            obj.init_template();
        }
    }

    #[glib::derived_properties]
    impl ObjectImpl for CardListHeader {
        fn constructed(&self) {
            self.parent_constructed();
            super::add_overflow(&self.shared_pills, &self.overflow);
            self.set_tags(Vec::new());
        }
    }
    impl WidgetImpl for CardListHeader {}
    impl BoxImpl for CardListHeader {}
}

glib::wrapper! {
    pub struct CardListHeader(ObjectSubclass<imp::CardListHeader>)
        @extends gtk::Box, gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget, gtk::Orientable;
}

/// Puts `overflow`, the "+N" pill of wrapping `pills` that show at most
/// [`HEADER_TAGS`] tags, in the pills' wrap box after them, styled as a
/// pill ([`show_capped`] fills it). Also for the deck page's group headers.
pub fn add_overflow(pills: &crate::widgets::TagPills, overflow: &gtk::Label) {
    use gtk::prelude::*;
    overflow.set_use_markup(false);
    overflow.set_css_classes(&["tag-pill", "caption", "numeric"]);
    overflow.set_valign(gtk::Align::Center);
    pills.set_end_child(Some(overflow.upcast_ref::<gtk::Widget>()));
}

/// Shows `tags` in wrapping `pills`: at most [`HEADER_TAGS`], then the
/// `overflow` pill ("+34", [`add_overflow`]) whose tooltip names the rest
/// and whose accessible label says them in words. A header never scrolls
/// sideways and a card can have thousands of tags (ADR 0050).
pub fn show_capped(pills: &crate::widgets::TagPills, overflow: &gtk::Label, tags: &[String]) {
    use crate::widgets::tag_pills::text::{overflow_text, overflow_tooltip, split, spoken};
    use gtk::prelude::*;
    let (shown, hidden) = split(tags.len(), Some(HEADER_TAGS));
    pills.set_tags(&tags[..shown]);
    overflow.set_visible(hidden > 0);
    if hidden > 0 {
        let rest = &tags[shown..];
        overflow.set_label(&overflow_text(hidden));
        overflow.set_tooltip_text(Some(&overflow_tooltip(rest)));
        // A screen reader gets the names, not "+34": the label says the
        // tags in words (at most 20, then how many more).
        let spoken = spoken(rest).unwrap_or_default();
        overflow.update_property(&[gtk::accessible::Property::Label(&spoken)]);
    }
}

impl Default for CardListHeader {
    fn default() -> Self {
        glib::Object::new()
    }
}

#[cfg(test)]
mod tests {
    use adw::subclass::prelude::*;
    use gtk::prelude::*;

    use super::*;

    #[test]
    fn the_header_shows_the_shared_tags_or_hides() {
        crate::testing::with_widgets(|| {
            let header = CardListHeader::default();
            let imp = header.imp();
            assert!(!header.is_visible());
            header.set_tags(vec!["Définition".to_owned(), "SIC3101".to_owned()]);
            assert!(header.is_visible());
            assert_eq!(imp.shared_pills.tags(), ["Définition", "SIC3101"]);
            // Every name, wrapping: no "+N" above the list.
            assert!(!imp.shared_pills.single_line());
            assert!(!imp.overflow.is_visible());
            // Past six tags: six pills and "+N", which names the rest.
            let many: Vec<String> = (0..40).map(|i| format!("tag {i}")).collect();
            header.set_tags(many);
            assert_eq!(imp.shared_pills.tags().len(), HEADER_TAGS as usize);
            assert!(imp.overflow.is_visible());
            assert_eq!(imp.overflow.label(), "+34");
            assert!(
                imp.overflow
                    .tooltip_text()
                    .is_some_and(|t| t.starts_with("\u{2068}tag 6\u{2069}"))
            );
            assert_eq!(header.tags().len(), 40);
            assert!(gtk::test_accessible_has_property(
                &imp.overflow,
                gtk::AccessibleProperty::Label
            ));
            header.set_tags(Vec::new());
            assert!(!header.is_visible());
        });
    }
}
