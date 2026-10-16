//! `MathView`: one formula, drawn as vectors (ADR 0021).
//!
//! The widget measures the formula's laid-out size at the current body font
//! size × [`MATH_SCALE`] and draws it into a cairo node. Cairo nodes are
//! recorded as drawing commands and rasterised by the renderer at the
//! surface's scale, so the glyph outlines stay sharp on a `HiDPI` display
//! (the spike's scale-2 screenshots; the GL renderer is on the checklist).
//!
//! The colour is the widget's CSS `color`, read when it draws: GTK's
//! `gtk_widget_get_color` is meant for exactly this ("snapshot
//! implementations that need to do custom drawing with the foreground
//! color"), and the default `css_changed` handling queues a redraw when a
//! default CSS property such as `color` changes (GTK docs, `css_changed`),
//! so a light/dark switch draws the formula again, laid out in the new
//! colour from the cache ([`math::formula`]).
//!
//! The widget is not focusable. Its accessible role is Math and its label
//! is the LaTeX source.

use gtk::glib;
use gtk::prelude::*;
use gtk::subclass::prelude::*;
use ratex_types::Color;

use super::math::{self, MATH_SCALE, ceil_px, f32_px};

glib::wrapper! {
    pub struct MathView(ObjectSubclass<imp::MathView>)
        @extends gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}

/// The size of a formula in pixels.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Size {
    pub width: i32,
    /// Height plus depth.
    pub height: i32,
}

impl MathView {
    /// A view of `source`, in the display style for `$$…$$`. The source must
    /// have been accepted by [`math::Budget::place`].
    pub fn new(source: &str, display: bool) -> Self {
        let view: Self = glib::Object::new();
        view.imp().source.replace(source.to_owned());
        view.imp().display.set(display);
        view.update_property(&[gtk::accessible::Property::Label(source)]);
        view
    }

    /// The pixel size of the formula now (0 × 0 if it cannot be laid out,
    /// which `place` has ruled out).
    pub fn size(&self) -> Size {
        self.imp().size()
    }
}

/// The body font's size in pixels at `widget`: its Pango font size and the
/// display resolution (`gtk-xft-dpi`, which GTK also gives Pango, and which
/// carries the text scaling factor).
pub fn em_px(widget: &impl IsA<gtk::Widget>) -> f64 {
    let dpi = math::dpi_from_setting(widget.settings().gtk_xft_dpi());
    let px = widget
        .pango_context()
        .font_description()
        .map_or(0.0, |d| math::font_px(d.size(), d.is_size_absolute(), dpi));
    if px > 0.0 { px } else { 15.0 }
}

fn to_color(rgba: &gtk::gdk::RGBA) -> Color {
    Color::new(rgba.red(), rgba.green(), rgba.blue(), rgba.alpha())
}

mod imp {
    use std::cell::{Cell, RefCell};
    use std::panic::{AssertUnwindSafe, catch_unwind};
    use std::rc::Rc;

    use gtk::prelude::*;
    use gtk::subclass::prelude::*;
    use gtk::{glib, graphene};
    use ratex_cairo::{CairoOptions, render_to_cairo};
    use ratex_types::{Color, DisplayList};

    use super::{MATH_SCALE, Size, ceil_px, em_px, f32_px, math, to_color};

    #[derive(Debug, Default)]
    pub struct MathView {
        pub source: RefCell<String>,
        pub display: Cell<bool>,
        /// The display list in the colour it was made for. The shared cache
        /// (`math::formula`) holds fewer entries than a review page can show
        /// at once (both sides, 200 formulas each, in two colours), so each
        /// view keeps its own: measuring, allocating and drawing, which GTK
        /// does many times, then never lay out again.
        memo: RefCell<Option<(Color, Rc<DisplayList>)>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for MathView {
        const NAME: &'static str = "RappelMathView";
        type Type = super::MathView;
        type ParentType = gtk::Widget;

        fn class_init(klass: &mut Self::Class) {
            klass.set_css_name("math");
            klass.set_accessible_role(gtk::AccessibleRole::Math);
        }
    }

    impl ObjectImpl for MathView {}

    impl WidgetImpl for MathView {
        fn measure(&self, orientation: gtk::Orientation, _for_size: i32) -> (i32, i32, i32, i32) {
            let size = self.size();
            let pixels = match orientation {
                gtk::Orientation::Horizontal => size.width,
                _ => size.height,
            };
            (pixels, pixels, -1, -1)
        }

        fn snapshot(&self, snapshot: &gtk::Snapshot) {
            let Some(list) = self.list() else { return };
            let font_size = self.font_size();
            let size = self.size();
            if size.width == 0 || size.height == 0 {
                return;
            }
            let bounds = graphene::Rect::new(
                0.0,
                0.0,
                f32_px(f64::from(size.width)),
                f32_px(f64::from(size.height)),
            );
            let cr = snapshot.append_cairo(&bounds);
            let options = CairoOptions {
                font_size,
                padding: 0.0,
                font_dir: None,
            };
            // The vendored drawing code has an `expect`; like the layout, a
            // bug there must not take the window down.
            let drawn = catch_unwind(AssertUnwindSafe(|| render_to_cairo(&cr, &list, &options)));
            match drawn {
                Ok(Ok(())) => {}
                Ok(Err(error)) => tracing::debug!(%error, "could not draw a formula"),
                Err(_) => tracing::warn!("drawing a formula panicked"),
            }
        }
    }

    impl MathView {
        fn font_size(&self) -> f64 {
            em_px(&*self.obj()) * MATH_SCALE
        }

        /// The display list in the current colour, from the cache.
        fn list(&self) -> Option<Rc<DisplayList>> {
            let color = to_color(&self.obj().color());
            if let Some((made_for, list)) = &*self.memo.borrow()
                && *made_for == color
            {
                return Some(Rc::clone(list));
            }
            let list = math::formula(&self.source.borrow(), self.display.get(), color).ok()?;
            self.memo.replace(Some((color, Rc::clone(&list))));
            Some(list)
        }

        /// The height of the formula above its baseline, in pixels.
        pub fn baseline(&self) -> f64 {
            self.list()
                .map_or(0.0, |list| list.height * self.font_size())
        }

        pub fn size(&self) -> Size {
            let Some(list) = self.list() else {
                return Size {
                    width: 0,
                    height: 0,
                };
            };
            let font_size = self.font_size();
            Size {
                width: ceil_px(list.width * font_size),
                height: ceil_px((list.height + list.depth) * font_size),
            }
        }
    }
}

/// The height of a formula above its baseline, in pixels: where it sits
/// relative to the text baseline in a line.
pub fn baseline_px(view: &MathView) -> f64 {
    view.imp().baseline()
}
