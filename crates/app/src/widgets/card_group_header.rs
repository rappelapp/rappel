//! `CardGroupHeader`: the header of a group of cards on the deck page
//! (ADR 0079), made by the list view's `header-factory` for each group on
//! screen. Layout in `data/card_group_header.blp`; texts in
//! `card_groups::text`.

use adw::prelude::*;
use adw::subclass::prelude::*;
use gtk::glib;

use super::card_groups::text;
use super::card_list_header::show_capped;

mod imp {
    use adw::prelude::*;
    use adw::subclass::prelude::*;
    use gtk::glib;

    use crate::widgets::TagPills;

    #[derive(Debug, Default, gtk::CompositeTemplate)]
    #[template(resource = "/io/github/rappelapp/Rappel/card_group_header.ui")]
    pub struct CardGroupHeader {
        #[template_child]
        pub title_label: TemplateChild<gtk::Label>,
        #[template_child]
        pub group_pills: TemplateChild<TagPills>,
        #[template_child]
        pub count_label: TemplateChild<gtk::Label>,
        /// The "+N" pill after the pills.
        pub overflow: gtk::Label,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for CardGroupHeader {
        const NAME: &'static str = "RappelCardGroupHeader";
        type Type = super::CardGroupHeader;
        type ParentType = gtk::Box;

        fn class_init(klass: &mut Self::Class) {
            TagPills::ensure_type();
            klass.bind_template();
            // Read as one heading, with the label `show` gives it, not
            // as its labels one by one (GtkAccessibleRole docs).
            klass.set_accessible_role(gtk::AccessibleRole::Heading);
        }

        fn instance_init(obj: &glib::subclass::InitializingObject<Self>) {
            obj.init_template();
        }
    }

    impl ObjectImpl for CardGroupHeader {
        fn constructed(&self) {
            self.parent_constructed();
            crate::widgets::card_list_header::add_overflow(&self.group_pills, &self.overflow);
            // The header's own label says it whole: its parts are hidden
            // from assistive technologies, so it is read once
            // (`GTK_ACCESSIBLE_STATE_HIDDEN`, GtkAccessible docs).
            for part in [
                self.title_label.upcast_ref::<gtk::Widget>(),
                self.group_pills.upcast_ref(),
                self.count_label.upcast_ref(),
                self.overflow.upcast_ref(),
            ] {
                part.update_state(&[gtk::accessible::State::Hidden(true)]);
            }
        }
    }
    impl WidgetImpl for CardGroupHeader {}
    impl BoxImpl for CardGroupHeader {}
}

glib::wrapper! {
    pub struct CardGroupHeader(ObjectSubclass<imp::CardGroupHeader>)
        @extends gtk::Box, gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget, gtk::Orientable;
}

impl Default for CardGroupHeader {
    fn default() -> Self {
        glib::Object::new()
    }
}

impl CardGroupHeader {
    /// Shows a group of `len` cards whose other tags are `tags` (names on
    /// one line each); `shared` is whether the list has shared tags.
    pub fn show(&self, tags: &[String], shared: bool, len: u32) {
        let imp = self.imp();
        imp.title_label
            .set_label(&text::title(!tags.is_empty(), shared));
        // Without pills the title takes the room, so the count stays at
        // the end.
        imp.title_label.set_hexpand(tags.is_empty());
        imp.group_pills.set_visible(!tags.is_empty());
        show_capped(&imp.group_pills, &imp.overflow, tags);
        imp.count_label.set_label(&text::count(len));
        self.update_property(&[gtk::accessible::Property::Label(&text::spoken(
            tags, shared, len,
        ))]);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_header_shows_its_group() {
        crate::testing::with_widgets(|| {
            let header = CardGroupHeader::default();
            let imp = header.imp();
            header.show(&["Analyse".to_owned(), "MAT3101".to_owned()], true, 7);
            assert_eq!(imp.title_label.label(), "Also");
            assert!(imp.group_pills.is_visible());
            assert_eq!(imp.group_pills.tags(), ["Analyse", "MAT3101"]);
            assert_eq!(imp.count_label.label(), "7 cards");
            assert!(gtk::test_accessible_has_property(
                &header,
                gtk::AccessibleProperty::Label
            ));
            assert_eq!(header.accessible_role(), gtk::AccessibleRole::Heading);
            assert!(gtk::test_accessible_has_state(
                &*imp.count_label,
                gtk::AccessibleState::Hidden
            ));
            header.show(&[], true, 14);
            assert_eq!(imp.title_label.label(), "Only the shared tags");
            assert!(!imp.group_pills.is_visible());
            assert_eq!(imp.count_label.label(), "14 cards");
            // Thousands of tags: six pills and "+N".
            let many: Vec<String> = (0..50).map(|i| format!("t{i}")).collect();
            header.show(&many, false, 2);
            assert_eq!(imp.title_label.label(), "Tagged");
            assert_eq!(imp.group_pills.tags().len(), 6);
            assert_eq!(imp.overflow.label(), "+44");
        });
    }
}
