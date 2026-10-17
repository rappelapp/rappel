//! `TagPills`: a card's tags as small rounded pills, neutral (the text
//! colour at a low opacity, `.tag-pill` in `data/style.css`) so they read in
//! light, dark and high-contrast styles. Layout in `data/tag_pills.blp`.
//!
//! Two forms, by the `single-line` property:
//! - wrapping (the default): every tag, in an `AdwWrapBox` that breaks the
//!   pills onto more lines when the width runs out;
//! - single line, for list rows: at most `max-shown` pills, then one "+N"
//!   pill for the rest, whose tooltip names them.
//!
//! Built to be bound often: a recycled list row calls [`TagPills::set_tags`]
//! on every bind, so the pill labels are kept and relabelled, never made
//! again, and nothing is done when the tags are the ones already shown.
//! Pills are plain labels, never focusable; a list row says its tags in its
//! own accessible label (`text::spoken`).

pub mod text;

use adw::prelude::*;
use adw::subclass::prelude::*;
use gtk::glib;

mod imp {
    use std::cell::{Cell, RefCell};

    use adw::prelude::*;
    use adw::subclass::prelude::*;
    use gtk::glib;

    #[derive(Debug, gtk::CompositeTemplate, glib::Properties)]
    #[properties(wrapper_type = super::TagPills)]
    #[template(resource = "/io/github/rsmirke/Rappel/tag_pills.ui")]
    pub struct TagPills {
        #[template_child]
        pub wrap_box: TemplateChild<adw::WrapBox>,
        #[template_child]
        pub line_box: TemplateChild<gtk::Box>,
        /// One line with a "+N" pill, instead of wrapping.
        #[property(get, set = Self::set_single_line, explicit_notify)]
        pub single_line: Cell<bool>,
        /// The most pills on a single line before the "+N" pill.
        #[property(get, set = Self::set_max_shown, explicit_notify)]
        pub max_shown: Cell<u32>,
        /// The tag names shown, each already one line.
        pub names: RefCell<Vec<String>>,
        /// The pill labels of the box in use, kept for the next tags.
        pub pills: RefCell<Vec<gtk::Label>>,
        /// The "+N" pill, made the first time it is needed.
        pub overflow: RefCell<Option<gtk::Label>>,
    }

    impl Default for TagPills {
        fn default() -> Self {
            Self {
                wrap_box: TemplateChild::default(),
                line_box: TemplateChild::default(),
                single_line: Cell::new(false),
                max_shown: Cell::new(super::text::ROW_PILLS),
                names: RefCell::default(),
                pills: RefCell::default(),
                overflow: RefCell::default(),
            }
        }
    }

    impl TagPills {
        fn set_single_line(&self, single_line: bool) {
            if self.single_line.replace(single_line) == single_line {
                return;
            }
            // The pills are in the box being left: start again in the other.
            self.obj().clear_pills(!single_line);
            self.wrap_box.set_visible(!single_line);
            self.line_box.set_visible(single_line);
            self.obj().refresh();
            self.obj().notify_single_line();
        }

        fn set_max_shown(&self, max_shown: u32) {
            if self.max_shown.replace(max_shown) == max_shown {
                return;
            }
            self.obj().refresh();
            self.obj().notify_max_shown();
        }
    }

    #[glib::object_subclass]
    impl ObjectSubclass for TagPills {
        const NAME: &'static str = "RappelTagPills";
        type Type = super::TagPills;
        type ParentType = gtk::Box;

        fn class_init(klass: &mut Self::Class) {
            klass.bind_template();
        }

        fn instance_init(obj: &glib::subclass::InitializingObject<Self>) {
            obj.init_template();
        }
    }

    #[glib::derived_properties]
    impl ObjectImpl for TagPills {}
    impl WidgetImpl for TagPills {}
    impl BoxImpl for TagPills {}
}

glib::wrapper! {
    pub struct TagPills(ObjectSubclass<imp::TagPills>)
        @extends gtk::Box, gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget, gtk::Orientable;
}

impl Default for TagPills {
    fn default() -> Self {
        glib::Object::new()
    }
}

impl TagPills {
    /// Shows `names` (each already one line, `text::pill_text`). Cheap
    /// when they are the tags already shown, and otherwise only relabels
    /// the kept pills.
    pub fn set_tags(&self, names: &[String]) {
        let imp = self.imp();
        if imp.names.borrow().as_slice() == names {
            return;
        }
        imp.names.replace(names.to_vec());
        self.refresh();
    }

    /// The tag names shown (all of them, the ones in the "+N" pill too).
    #[cfg(test)]
    pub fn tags(&self) -> Vec<String> {
        self.imp().names.borrow().clone()
    }

    /// Every pill label made so far, the "+N" pill last, shown or not.
    #[cfg(test)]
    pub fn pill_widgets(&self) -> Vec<gtk::Label> {
        let imp = self.imp();
        let mut all = imp.pills.borrow().clone();
        all.extend(imp.overflow.borrow().clone());
        all
    }

    /// The labels of the pills on screen, in order, the "+N" pill last.
    #[cfg(test)]
    fn shown_texts(&self) -> Vec<String> {
        let imp = self.imp();
        let mut texts: Vec<String> = imp
            .pills
            .borrow()
            .iter()
            .filter(|p| p.is_visible())
            .map(|p| p.label().to_string())
            .collect();
        if let Some(overflow) = imp.overflow.borrow().as_ref()
            && overflow.is_visible()
        {
            texts.push(overflow.label().to_string());
        }
        texts
    }

    /// Lays the pills out for the current names, reusing the labels made
    /// before; the ones not needed are hidden, not destroyed.
    fn refresh(&self) {
        let imp = self.imp();
        let single_line = imp.single_line.get();
        let names = imp.names.borrow();
        let (shown, hidden) = text::split(names.len(), single_line.then(|| imp.max_shown.get()));
        let mut pills = imp.pills.borrow_mut();
        while pills.len() < shown {
            let pill = pill_label();
            match self.overflow_label() {
                // Before the "+N" pill, which stays last.
                Some(overflow) if single_line => {
                    imp.line_box
                        .insert_child_after(&pill, overflow.prev_sibling().as_ref());
                }
                _ => self.container_append(&pill),
            }
            pills.push(pill);
        }
        for (i, pill) in pills.iter().enumerate() {
            match names.get(i).filter(|_| i < shown) {
                Some(name) => {
                    pill.set_label(name);
                    // `set_tooltip_text` is plain text, like the label.
                    pill.set_tooltip_text(text::is_cut(name).then_some(name.as_str()));
                    pill.set_visible(true);
                }
                None => pill.set_visible(false),
            }
        }
        drop(pills);
        if hidden > 0 {
            let overflow = self.overflow_label().unwrap_or_else(|| {
                let overflow = pill_label();
                overflow.add_css_class("numeric");
                overflow.set_ellipsize(gtk::pango::EllipsizeMode::None);
                self.container_append(&overflow);
                imp.overflow.replace(Some(overflow.clone()));
                overflow
            });
            overflow.set_label(&text::overflow_text(hidden));
            overflow.set_tooltip_text(Some(&text::overflow_tooltip(&names[shown..])));
            overflow.set_visible(true);
        } else if let Some(overflow) = self.overflow_label() {
            overflow.set_visible(false);
        }
    }

    fn overflow_label(&self) -> Option<gtk::Label> {
        self.imp().overflow.borrow().clone()
    }

    fn container_append(&self, pill: &gtk::Label) {
        let imp = self.imp();
        if imp.single_line.get() {
            imp.line_box.append(pill);
        } else {
            imp.wrap_box.append(pill);
        }
    }

    /// Removes every pill, from the single line if `in_line`, else from
    /// the wrap box.
    fn clear_pills(&self, in_line: bool) {
        let imp = self.imp();
        let pills = imp.pills.take();
        let overflow = imp.overflow.take();
        for pill in pills.iter().chain(overflow.as_ref()) {
            if in_line {
                imp.line_box.remove(pill);
            } else {
                imp.wrap_box.remove(pill);
            }
        }
    }
}

/// One pill: a plain label, cut with "…" past [`text::PILL_CHARS`].
fn pill_label() -> gtk::Label {
    gtk::Label::builder()
        .use_markup(false)
        .single_line_mode(true)
        .ellipsize(gtk::pango::EllipsizeMode::End)
        .max_width_chars(text::PILL_CHARS)
        .valign(gtk::Align::Center)
        .css_classes(["tag-pill", "caption"])
        .build()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(list: &[&str]) -> Vec<String> {
        list.iter().map(|&n| n.to_owned()).collect()
    }

    #[test]
    fn pills_are_reused_and_the_rest_counted() {
        crate::testing::with_widgets(|| {
            let pills = TagPills::default();
            pills.set_single_line(true);
            pills.set_max_shown(2);
            pills.set_tags(&names(&["a", "b", "c", "d"]));
            assert_eq!(pills.shown_texts(), ["a", "b", "+2"]);
            let first = pills.imp().pills.borrow()[0].clone();
            pills.set_tags(&names(&["x"]));
            assert_eq!(pills.shown_texts(), ["x"]);
            // The same label, relabelled: nothing is made again on a bind.
            assert_eq!(pills.imp().pills.borrow()[0], first);
            pills.set_max_shown(3);
            pills.set_tags(&names(&["a", "b", "c", "d", "e"]));
            assert_eq!(pills.shown_texts(), ["a", "b", "c", "+2"]);
            // The "+N" pill stays last after a pill is added before it.
            let last = pills
                .imp()
                .line_box
                .last_child()
                .and_downcast::<gtk::Label>();
            assert_eq!(last.map(|l| l.label().to_string()).as_deref(), Some("+2"));
            pills.set_tags(&[]);
            assert_eq!(pills.shown_texts(), Vec::<String>::new());
        });
    }

    #[test]
    fn wrapping_pills_show_every_tag() {
        crate::testing::with_widgets(|| {
            let pills = TagPills::default();
            let many: Vec<String> = (0..10).map(|i| format!("tag {i}")).collect();
            pills.set_tags(&many);
            assert_eq!(pills.shown_texts(), many);
            // Switching form moves the pills to the other box.
            pills.set_single_line(true);
            assert_eq!(pills.shown_texts(), ["tag 0", "tag 1", "tag 2", "+7"]);
            assert!(pills.imp().wrap_box.first_child().is_none());
            // Back to wrapping: the single line, "+N" pill included, is
            // emptied, and every tag wraps again.
            pills.set_single_line(false);
            assert_eq!(pills.shown_texts(), many);
            assert!(pills.imp().line_box.first_child().is_none());
            // And to a single line again, with a new "+N" pill.
            pills.set_single_line(true);
            assert_eq!(pills.shown_texts(), ["tag 0", "tag 1", "tag 2", "+7"]);
            assert!(pills.imp().wrap_box.first_child().is_none());
            let last = pills
                .imp()
                .line_box
                .last_child()
                .and_downcast::<gtk::Label>();
            assert_eq!(last.map(|l| l.label().to_string()).as_deref(), Some("+7"));
        });
    }
}
