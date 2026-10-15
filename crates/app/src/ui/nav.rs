//! The `AdwNavigationView` that pages are pushed onto.
//!
//! Focus after a push: a `grab_focus()` before or during the push is lost,
//! and the view then focuses the first focusable widget (usually Back).
//! Set the focus in the pushed page's `shown` handler instead.

use adw::prelude::*;

/// The navigation view that contains `widget`, or `None` after logging:
/// every page lives in the window's view, so that is a bug.
pub fn navigation_view(widget: &impl IsA<gtk::Widget>) -> Option<adw::NavigationView> {
    let view = widget
        .ancestor(adw::NavigationView::static_type())
        .and_downcast::<adw::NavigationView>();
    if view.is_none() {
        tracing::error!("{} is not inside a navigation view", widget.type_().name());
    }
    view
}

/// Goes back from the page that contains `widget`.
pub fn pop(widget: &impl IsA<gtk::Widget>) {
    if let Some(view) = navigation_view(widget) {
        view.pop();
    }
}
