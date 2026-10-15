//! Main application window. Layout lives in `data/window.blp`.

use adw::subclass::prelude::*;
use gtk::{gio, glib};

mod imp {
    use adw::subclass::prelude::*;
    use gtk::glib;

    #[derive(Debug, Default, gtk::CompositeTemplate)]
    #[template(resource = "/io/github/rsmirke/Cards/window.ui")]
    pub struct CardsWindow {
        #[template_child]
        pub toast_overlay: TemplateChild<adw::ToastOverlay>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for CardsWindow {
        const NAME: &'static str = "CardsWindow";
        type Type = super::CardsWindow;
        type ParentType = adw::ApplicationWindow;

        fn class_init(klass: &mut Self::Class) {
            klass.bind_template();
        }

        fn instance_init(obj: &glib::subclass::InitializingObject<Self>) {
            obj.init_template();
        }
    }

    impl ObjectImpl for CardsWindow {}
    impl WidgetImpl for CardsWindow {}
    impl WindowImpl for CardsWindow {}
    impl ApplicationWindowImpl for CardsWindow {}
    impl AdwApplicationWindowImpl for CardsWindow {}
}

glib::wrapper! {
    pub struct CardsWindow(ObjectSubclass<imp::CardsWindow>)
        @extends adw::ApplicationWindow, gtk::ApplicationWindow, gtk::Window, gtk::Widget,
        @implements gio::ActionGroup, gio::ActionMap, gtk::Accessible, gtk::Buildable,
                    gtk::ConstraintTarget, gtk::Native, gtk::Root, gtk::ShortcutManager;
}

impl CardsWindow {
    pub fn new(app: &adw::Application) -> Self {
        glib::Object::builder().property("application", app).build()
    }

    /// Shows `toast` over the window content.
    pub fn add_toast(&self, toast: adw::Toast) {
        self.imp().toast_overlay.add_toast(toast);
    }
}
