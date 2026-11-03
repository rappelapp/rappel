//! `SelectionHeader`: the header of selection mode, shared by All Cards and
//! a deck's page (ADR 0065). Select All at the start, how many cards are
//! chosen as the title, Cancel at the end (HIG, "Selection mode"). Layout in
//! `data/selection_header.blp`; its buttons activate the page's
//! `selection.*` actions, and `ui::selection` shows it in place of the
//! page's own header and sets its title.

use adw::subclass::prelude::*;
use gtk::glib;

mod imp {
    use std::cell::{Cell, RefCell};

    use adw::prelude::*;
    use adw::subclass::prelude::*;
    use gtk::glib;

    #[derive(Debug, Default, gtk::CompositeTemplate, glib::Properties)]
    #[properties(wrapper_type = super::SelectionHeader)]
    #[template(resource = "/io/github/rsmirke/Rappel/selection_header.ui")]
    pub struct SelectionHeader {
        #[template_child]
        pub title: TemplateChild<adw::WindowTitle>,
        #[template_child]
        pub select_all_button: TemplateChild<gtk::Button>,
        #[template_child]
        pub select_all_content: TemplateChild<adw::ButtonContent>,
        #[template_child]
        pub cancel_button: TemplateChild<gtk::Button>,
        /// A narrow window: Select All keeps only its icon.
        #[property(get, set)]
        pub compact: Cell<bool>,
        /// Select All's word, as the layout gives it (translated).
        pub select_all_word: RefCell<String>,
        /// The title in full and as the bare count, and which one shows:
        /// the count while the header is narrower than
        /// `text::TIGHT_HEADER_WIDTH`.
        pub full_title: RefCell<String>,
        pub tight_title: RefCell<String>,
        pub tight: Cell<bool>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for SelectionHeader {
        const NAME: &'static str = "RappelSelectionHeader";
        type Type = super::SelectionHeader;
        type ParentType = adw::Bin;

        fn class_init(klass: &mut Self::Class) {
            klass.bind_template();
        }

        fn instance_init(obj: &glib::subclass::InitializingObject<Self>) {
            obj.init_template();
        }
    }

    #[glib::derived_properties]
    impl ObjectImpl for SelectionHeader {
        fn constructed(&self) {
            self.parent_constructed();
            self.select_all_word
                .replace(self.select_all_content.label().to_string());
            let header = self.obj();
            // The title follows the window's width (ADR 0071).
            crate::ui::breakpoint::follow_window_width(
                &*header,
                crate::ui::selection::text::TIGHT_HEADER_WIDTH,
                glib::clone!(
                    #[weak]
                    header,
                    move |tight| header.set_tight(tight)
                ),
            );
            header.connect_compact_notify(|header| {
                let imp = header.imp();
                let word = if header.compact() {
                    String::new()
                } else {
                    imp.select_all_word.borrow().clone()
                };
                imp.select_all_content.set_label(&word);
            });
        }
    }

    impl WidgetImpl for SelectionHeader {
        fn root(&self) {
            // Roots the children first (`gtk_widget_real_root`), so the
            // `AdwButtonContent` has named its button by then.
            self.parent_root();
            super::super::selection_bar::name_by_own_label(&self.select_all_button);
        }
    }

    impl BinImpl for SelectionHeader {}
}

glib::wrapper! {
    pub struct SelectionHeader(ObjectSubclass<imp::SelectionHeader>)
        @extends adw::Bin, gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}

impl SelectionHeader {
    /// Sets the title ("3 Selected") and the shorter one for a narrow
    /// header ("3"), which has the full one as its tooltip.
    pub fn set_titles(&self, full: &str, tight: &str) {
        let imp = self.imp();
        imp.full_title.replace(full.to_owned());
        imp.tight_title.replace(tight.to_owned());
        self.show_title();
    }

    /// A window narrower than `text::TIGHT_HEADER_WIDTH` shows the bare
    /// count as the title.
    fn set_tight(&self, tight: bool) {
        if tight != self.imp().tight.replace(tight) {
            self.show_title();
        }
    }

    /// Shows the title that fits the header's width. A screen reader hears
    /// the whole title however it is drawn: the accessible label goes on the
    /// label that draws it, not on the `AdwWindowTitle`. That widget sets no
    /// role of its own, so it has GTK's default, `GTK_ACCESSIBLE_ROLE_GENERIC`,
    /// "a nameless container that has no semantic meaning of its own"
    /// (Gtk-4.0.gir; checked in the test below): a label set on it is not
    /// what Orca reads, which is the inner label's own name, its text ("30")
    /// unless an accessible label is set on it (the ARIA name computation
    /// GTK follows: `aria-label` before the content).
    fn show_title(&self) {
        use gtk::prelude::{AccessibleExtManual, Cast, WidgetExt};
        let imp = self.imp();
        let full = imp.full_title.borrow();
        if imp.tight.get() {
            imp.title.set_title(&imp.tight_title.borrow());
            imp.title.set_tooltip_text(Some(&full));
        } else {
            imp.title.set_title(&full);
            imp.title.set_tooltip_text(None);
        }
        let label = [gtk::accessible::Property::Label(&full)];
        match title_label(&imp.title) {
            Some(inner) => inner.update_property(&label),
            // Not found (a libadwaita that draws its title another way):
            // the widget itself, the best there is.
            None => imp
                .title
                .upcast_ref::<gtk::Widget>()
                .update_property(&label),
        }
    }

    /// The title shown.
    #[cfg(test)]
    pub fn title(&self) -> String {
        self.imp().title.title().to_string()
    }

    /// Select All.
    #[cfg(test)]
    pub fn select_all_button(&self) -> gtk::Button {
        self.imp().select_all_button.get()
    }

    /// Cancel.
    pub fn cancel_button(&self) -> gtk::Button {
        self.imp().cancel_button.get()
    }
}

/// The label inside `title` that draws its title: the `GtkLabel` with the
/// style class `title` (libadwaita's `adw-window-title.ui` has a `title` and
/// a `subtitle` label; found by walking the children, as libadwaita gives
/// no getter for it). `None` if there is none.
fn title_label(title: &adw::WindowTitle) -> Option<gtk::Label> {
    use gtk::prelude::{Cast, WidgetExt};
    let mut todo: Vec<gtk::Widget> = title.first_child().into_iter().collect();
    while let Some(widget) = todo.pop() {
        if widget.has_css_class("title")
            && let Some(label) = widget.downcast_ref::<gtk::Label>()
        {
            return Some(label.clone());
        }
        todo.extend(widget.first_child());
        todo.extend(widget.next_sibling());
    }
    None
}

#[cfg(test)]
mod tests {
    use gtk::prelude::WidgetExt;

    use super::*;

    #[test]
    fn a_narrow_header_keeps_only_the_icon_of_select_all() {
        crate::testing::with_widgets(|| {
            let header: SelectionHeader = glib::Object::new();
            let content = &header.imp().select_all_content;
            assert_eq!(content.label(), "Select _All");
            header.set_compact(true);
            assert_eq!(content.label(), "");
            header.set_compact(false);
            assert_eq!(content.label(), "Select _All");
            header.set_titles("2 Selected", "2");
            assert_eq!(header.title(), "2 Selected");
            // Narrow: the count, the whole title as its tooltip.
            header.set_tight(true);
            assert_eq!(header.title(), "2");
            assert_eq!(
                header.imp().title.tooltip_text().as_deref(),
                Some("2 Selected")
            );
            // And a new title keeps the state.
            header.set_titles("3 Selected", "3");
            assert_eq!(header.title(), "3");
            header.set_tight(false);
            assert_eq!(header.title(), "3 Selected");
            assert_eq!(header.imp().title.tooltip_text(), None);
        });
    }

    /// What a screen reader names is the inner label: `AdwWindowTitle` is a
    /// generic, nameless container, and the label that draws the title is
    /// found (where `show_title` puts the accessible label).
    #[test]
    fn the_accessible_label_goes_on_the_label_that_draws_the_title() {
        use gtk::prelude::{AccessibleExt, Cast};
        crate::testing::with_widgets(|| {
            let header: SelectionHeader = glib::Object::new();
            let title = header.imp().title.get();
            assert_eq!(
                title.upcast_ref::<gtk::Widget>().accessible_role(),
                gtk::AccessibleRole::Generic
            );
            header.set_titles("30 Selected", "30");
            header.set_tight(true);
            let inner = title_label(&title).expect("AdwWindowTitle draws its title in a label");
            assert_eq!(inner.label(), "30", "the label that draws the title");
            assert_eq!(inner.accessible_role(), gtk::AccessibleRole::Label);
            header.set_tight(false);
            assert_eq!(inner.label(), "30 Selected");
        });
    }
}
