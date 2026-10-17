//! One entry of a card's history in the Card Information dialog: an icon,
//! the result as a word, and when, how long and what it scheduled. Layout
//! in `data/history_row.blp`. The icon repeats what the title says, so the
//! screen reader skips it.

use adw::prelude::*;
use adw::subclass::prelude::*;
use gtk::glib;

mod imp {
    use adw::subclass::prelude::*;
    use gtk::glib;

    #[derive(Debug, Default, gtk::CompositeTemplate)]
    #[template(resource = "/io/github/rsmirke/Rappel/history_row.ui")]
    pub struct HistoryRow {
        #[template_child]
        pub icon: TemplateChild<gtk::Image>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for HistoryRow {
        const NAME: &'static str = "RappelHistoryRow";
        type Type = super::HistoryRow;
        type ParentType = adw::ActionRow;

        fn class_init(klass: &mut Self::Class) {
            klass.bind_template();
        }

        fn instance_init(obj: &glib::subclass::InitializingObject<Self>) {
            obj.init_template();
        }
    }

    impl ObjectImpl for HistoryRow {}
    impl WidgetImpl for HistoryRow {}
    impl ListBoxRowImpl for HistoryRow {}
    impl PreferencesRowImpl for HistoryRow {}
    impl ActionRowImpl for HistoryRow {}
}

glib::wrapper! {
    pub struct HistoryRow(ObjectSubclass<imp::HistoryRow>)
        @extends adw::ActionRow, adw::PreferencesRow, gtk::ListBoxRow, gtk::Widget,
        @implements gtk::Accessible, gtk::Actionable, gtk::Buildable, gtk::ConstraintTarget;
}

impl HistoryRow {
    /// A row with a symbolic icon (none for an empty name; `css_class` colours it with a theme
    /// style class, if not empty), a title and a subtitle.
    pub fn new(icon_name: &str, css_class: &str, title: &str, subtitle: &str) -> Self {
        let row: Self = glib::Object::new();
        row.set_title(title);
        row.set_subtitle(subtitle);
        let icon = &row.imp().icon;
        icon.set_visible(!icon_name.is_empty());
        icon.set_icon_name(Some(icon_name));
        if !css_class.is_empty() {
            icon.add_css_class(css_class);
        }
        icon.update_state(&[gtk::accessible::State::Hidden(true)]);
        row
    }
}
