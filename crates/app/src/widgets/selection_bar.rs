//! `SelectionBar`: the action bar of selection mode, shared by All Cards and
//! a deck's page (ADR 0065): Move…, Tags (Add Tags…, Remove Tag…), Suspend
//! or Unsuspend, Reset and Delete (HIG, "Selection mode": an action bar at
//! the bottom). Layout in `data/selection_bar.blp`; its buttons activate the
//! page's `selection.*` actions, and `ui::selection` reveals it and says
//! which way Suspend goes.

use adw::prelude::*;
use adw::subclass::prelude::*;
use gtk::glib;

use crate::ui::selection::text::{SuspendChoice, suspend_labels};

mod imp {
    use std::cell::{Cell, RefCell};

    use adw::prelude::*;
    use adw::subclass::prelude::*;
    use gtk::glib;

    #[derive(Debug, Default, gtk::CompositeTemplate, glib::Properties)]
    #[properties(wrapper_type = super::SelectionBar)]
    #[template(resource = "/io/github/rsmirke/Rappel/selection_bar.ui")]
    pub struct SelectionBar {
        #[template_child]
        pub action_bar: TemplateChild<gtk::ActionBar>,
        #[template_child]
        pub move_button: TemplateChild<gtk::Button>,
        #[template_child]
        pub move_content: TemplateChild<adw::ButtonContent>,
        #[template_child]
        pub tags_button: TemplateChild<gtk::MenuButton>,
        #[template_child]
        pub tags_content: TemplateChild<adw::ButtonContent>,
        #[template_child]
        pub suspend_button: TemplateChild<gtk::Button>,
        #[template_child]
        pub suspend_content: TemplateChild<adw::ButtonContent>,
        #[template_child]
        pub reset_button: TemplateChild<gtk::Button>,
        #[template_child]
        pub reset_content: TemplateChild<adw::ButtonContent>,
        #[template_child]
        pub delete_button: TemplateChild<gtk::Button>,
        #[template_child]
        pub delete_content: TemplateChild<adw::ButtonContent>,
        /// A narrow window: the buttons keep only their icons.
        #[property(get, set)]
        pub compact: Cell<bool>,
        /// The words of the buttons whose word does not change, as the
        /// layout gives them (translated).
        pub words: RefCell<Vec<(adw::ButtonContent, String)>>,
        /// Suspend's word now: "Suspend" or "Unsuspend".
        pub suspend_word: RefCell<String>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for SelectionBar {
        const NAME: &'static str = "RappelSelectionBar";
        type Type = super::SelectionBar;
        type ParentType = adw::Bin;

        fn class_init(klass: &mut Self::Class) {
            klass.bind_template();
        }

        fn instance_init(obj: &glib::subclass::InitializingObject<Self>) {
            obj.init_template();
        }
    }

    #[glib::derived_properties]
    impl ObjectImpl for SelectionBar {
        fn constructed(&self) {
            self.parent_constructed();
            self.words.replace(
                [
                    &*self.move_content,
                    &*self.tags_content,
                    &*self.reset_content,
                    &*self.delete_content,
                ]
                .into_iter()
                .map(|content| (content.clone(), content.label().to_string()))
                .collect(),
            );
            self.suspend_word
                .replace(self.suspend_content.label().to_string());
            self.obj()
                .connect_compact_notify(super::SelectionBar::show_words);
        }
    }

    impl WidgetImpl for SelectionBar {
        fn root(&self) {
            // Roots the children first (`gtk_widget_real_root`), so every
            // `AdwButtonContent` has named its button by then.
            self.parent_root();
            for button in [
                &*self.move_button,
                &*self.suspend_button,
                &*self.delete_button,
                &*self.reset_button,
            ] {
                super::name_by_own_label(button);
            }
        }
    }

    impl BinImpl for SelectionBar {}
}

glib::wrapper! {
    pub struct SelectionBar(ObjectSubclass<imp::SelectionBar>)
        @extends adw::Bin, gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}

impl SelectionBar {
    /// Shows or hides the bar (it slides in and out).
    pub fn set_revealed(&self, revealed: bool) {
        self.imp().action_bar.set_revealed(revealed);
    }

    /// Whether the bar is shown.
    #[cfg(test)]
    pub fn is_revealed(&self) -> bool {
        self.imp().action_bar.is_revealed()
    }

    /// Tags is a menu button, which does not follow its menu's actions.
    pub fn set_tags_sensitive(&self, sensitive: bool) {
        self.imp().tags_button.set_sensitive(sensitive);
    }

    /// Makes Suspend say what it will do to the chosen cards.
    pub fn show_suspend(&self, choice: SuspendChoice) {
        let imp = self.imp();
        let (label, accessible) = suspend_labels(choice);
        imp.suspend_button.set_tooltip_text(Some(&label));
        imp.suspend_button
            .update_property(&[gtk::accessible::Property::Label(&accessible)]);
        imp.suspend_content.set_icon_name(match choice {
            SuspendChoice::Suspend => "media-playback-pause-symbolic",
            SuspendChoice::Unsuspend => "media-playback-start-symbolic",
        });
        imp.suspend_word.replace(label);
        self.show_words();
    }

    /// The buttons' words, or none on a narrow window.
    fn show_words(&self) {
        let imp = self.imp();
        let compact = self.compact();
        for (content, word) in imp.words.borrow().iter() {
            content.set_label(if compact { "" } else { word });
        }
        let suspend = imp.suspend_word.borrow();
        imp.suspend_content
            .set_label(if compact { "" } else { suspend.as_str() });
    }

    /// The bar's buttons, for the tests.
    #[cfg(test)]
    pub fn buttons(&self) -> [gtk::Button; 4] {
        let imp = self.imp();
        [
            imp.move_button.get(),
            imp.suspend_button.get(),
            imp.delete_button.get(),
            imp.reset_button.get(),
        ]
    }

    /// Reset Progress, for the tests.
    #[cfg(test)]
    pub fn reset_button(&self) -> gtk::Button {
        self.imp().reset_button.get()
    }
}

/// Lets `button`'s own accessible label (Blueprint, or `suspend_labels`)
/// name it, not its `AdwButtonContent`'s text. That content makes its
/// label the button's mnemonic label when it is rooted
/// (`adw_button_content_root`, source read for libadwaita 1.9 and main),
/// which sets the button's labelled-by relation
/// (`gtk_widget_add_mnemonic_label`, source read for GTK 4.24), and
/// a labelled-by relation names a widget before its label property does
/// (`gtk_at_context_get_text_accumulate`, steps 2B and 2C of the ARIA
/// name computation, gtk/gtkatcontext.c, GTK 4.24). The button was then
/// named by its visible word ("Move…") and, on a phone, where the
/// breakpoint empties the words, not at all (observed over AT-SPI with
/// libadwaita 1.10.0 and GTK 4.24.1). Only the accessible relation is
/// reset: the mnemonic label stays, so a mnemonic such as Select All's
/// "_All" still activates the button.
pub(crate) fn name_by_own_label(button: &gtk::Button) {
    button.reset_relation(gtk::AccessibleRelation::LabelledBy);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_narrow_bar_keeps_only_the_icons_and_suspend_follows_the_cards() {
        crate::testing::with_widgets(|| {
            let bar: SelectionBar = glib::Object::new();
            let imp = bar.imp();
            assert_eq!(imp.move_content.label(), "Move…");
            bar.show_suspend(SuspendChoice::Unsuspend);
            assert_eq!(imp.suspend_content.label(), "Unsuspend");
            assert_eq!(
                imp.suspend_content.icon_name(),
                "media-playback-start-symbolic"
            );
            bar.set_compact(true);
            for content in [
                &imp.move_content,
                &imp.tags_content,
                &imp.suspend_content,
                &imp.reset_content,
                &imp.delete_content,
            ] {
                assert_eq!(content.label(), "");
            }
            // The choice changes while narrow: still no word.
            bar.show_suspend(SuspendChoice::Suspend);
            assert_eq!(imp.suspend_content.label(), "");
            assert_eq!(
                imp.suspend_button.tooltip_text().as_deref(),
                Some("Suspend")
            );
            bar.set_compact(false);
            assert_eq!(imp.suspend_content.label(), "Suspend");
            assert_eq!(imp.delete_content.label(), "Delete");
        });
    }
}
