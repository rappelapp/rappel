//! Following an `AdwBreakpoint` in code, for what its setters cannot do
//! (a property of the page that its rows follow, a style class).

use std::cell::RefCell;
use std::rc::Rc;

use gtk::glib;

use gtk::prelude::IsA;

/// Calls `f(true)` each time `breakpoint` starts to apply and `f(false)`
/// each time it stops. libadwaita emits `apply` and `unapply` as the
/// condition starts and stops matching, the first time too when the
/// widget is first sized, and after the breakpoint's setters (`AdwBreakpoint`
/// docs). `f` should hold its widget weakly (`glib::clone!` with `#[weak]`).
pub fn follow(breakpoint: &adw::Breakpoint, f: impl Fn(bool) + 'static) {
    let f = Rc::new(f);
    let applied = Rc::clone(&f);
    breakpoint.connect_apply(move |_| applied(true));
    breakpoint.connect_unapply(move |_| f(false));
}

/// Calls `f(true)` while the window `widget` is in is narrower than
/// `threshold` pixels, and `f(false)` otherwise, each time that changes
/// (and at the first layout). For what a breakpoint cannot do without
/// shadowing the page's other ones: `AdwBreakpointBin` applies only the
/// last matching breakpoint (its docs), so a third width on a page that has
/// two would have to repeat their setters. The surface's `layout` signal
/// comes with every resize (`GdkSurface` docs). A widget whose own
/// `size_allocate` could say it cannot: one with a layout manager, like
/// `AdwBin`, never has its vfunc called (`gtk_widget_allocate`). `f` should
/// hold its widget weakly.
///
/// `threshold` is in **pixels** of the window's surface, not `sp` (a
/// breakpoint's unit, which follows the font size): the signal gives pixels,
/// and 400 px is what the 360 px phones in question need. The handler is
/// on the surface a widget is realized on and is disconnected when it is
/// unrealized, so a widget that is shown again does not add a second one.
pub fn follow_window_width(
    widget: &impl IsA<gtk::Widget>,
    threshold: i32,
    f: impl Fn(bool) + 'static,
) {
    use gtk::prelude::*;
    let f = Rc::new(f);
    // `None` until the first layout, so the first one always calls `f`.
    let last = Rc::new(std::cell::Cell::new(None::<bool>));
    // The surface and the handler on it, while the widget is realized.
    let connected: Rc<RefCell<Option<(gtk::gdk::Surface, glib::SignalHandlerId)>>> = Rc::default();
    let held = Rc::clone(&connected);
    widget.connect_realize(move |widget| {
        let Some(surface) = widget.native().and_then(|native| native.surface()) else {
            return;
        };
        let (f, last) = (Rc::clone(&f), Rc::clone(&last));
        let id = surface.connect_layout(move |_, width, _| {
            let narrow = is_narrower(width, threshold);
            if last.replace(Some(narrow)) != Some(narrow) {
                f(narrow);
            }
        });
        if let Some((old, old_id)) = held.borrow_mut().replace((surface, id)) {
            old.disconnect(old_id);
        }
    });
    widget.connect_unrealize(move |_| {
        if let Some((surface, id)) = connected.borrow_mut().take() {
            surface.disconnect(id);
        }
    });
}

/// Whether a window `width` pixels wide is narrower than `threshold`.
fn is_narrower(width: i32, threshold: i32) -> bool {
    width < threshold
}

/// Makes `status` `compact` (libadwaita's style class for a status page in
/// a small space: less spacing) without its icon while `breakpoint`
/// applies, and gives `icon` back after, so a short window (a phone held
/// sideways, 640 × 360) shows the page's text and button rather than a
/// large icon (ADR 0045, 0052, 0058). A setter can neither add a style
/// class nor unset the icon, hence the code.
pub fn compact_status_page(
    breakpoint: &adw::Breakpoint,
    status: &adw::StatusPage,
    icon: &'static str,
) {
    use adw::prelude::*;
    let status = status.downgrade();
    follow(breakpoint, move |short| {
        if let Some(status) = status.upgrade() {
            if short {
                status.add_css_class("compact");
            } else {
                status.remove_css_class("compact");
            }
            status.set_icon_name((!short).then_some(icon));
        }
    });
}

/// Makes `widget` `compact` (a status page's smaller layout) while
/// `breakpoint` applies, keeping its icon: for a status page whose icon
/// the page changes as it goes (All Cards' empty states; ADR 0058).
pub fn compact_when(breakpoint: &adw::Breakpoint, widget: &impl IsA<gtk::Widget>) {
    use adw::prelude::*;
    let widget = widget.upcast_ref::<gtk::Widget>().downgrade();
    follow(breakpoint, move |short| {
        if let Some(widget) = widget.upgrade() {
            if short {
                widget.add_css_class("compact");
            } else {
                widget.remove_css_class("compact");
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;

    use adw::prelude::*;

    use super::*;

    #[test]
    fn apply_and_unapply_reach_the_closure() {
        crate::testing::with_widgets(|| {
            let breakpoint =
                adw::Breakpoint::new(adw::BreakpointCondition::parse("max-width: 400sp").unwrap());
            let seen = Rc::new(RefCell::new(Vec::new()));
            let sink = Rc::clone(&seen);
            follow(&breakpoint, move |on| sink.borrow_mut().push(on));
            breakpoint.emit_by_name::<()>("apply", &[]);
            breakpoint.emit_by_name::<()>("unapply", &[]);
            breakpoint.emit_by_name::<()>("apply", &[]);
            assert_eq!(*seen.borrow(), [true, false, true]);
        });
    }

    #[test]
    fn a_status_page_is_compact_without_its_icon_while_short() {
        crate::testing::with_widgets(|| {
            let breakpoint =
                adw::Breakpoint::new(adw::BreakpointCondition::parse("max-height: 450sp").unwrap());
            let status = adw::StatusPage::builder()
                .icon_name("dialog-warning-symbolic")
                .build();
            compact_status_page(&breakpoint, &status, "dialog-warning-symbolic");
            breakpoint.emit_by_name::<()>("apply", &[]);
            assert!(status.has_css_class("compact"));
            assert_eq!(status.icon_name(), None);
            breakpoint.emit_by_name::<()>("unapply", &[]);
            assert!(!status.has_css_class("compact"));
            assert_eq!(
                status.icon_name().as_deref(),
                Some("dialog-warning-symbolic")
            );
        });
    }
}

#[cfg(test)]
mod width_tests {
    use super::is_narrower;

    #[test]
    fn narrower_means_strictly_under_the_threshold() {
        assert!(is_narrower(360, 400));
        assert!(is_narrower(399, 400));
        assert!(!is_narrower(400, 400));
        assert!(!is_narrower(1000, 400));
    }
}
