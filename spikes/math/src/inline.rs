//! Inline B: one widget per paragraph. Pango lays out the text with an
//! `AttrShape` box per formula (logical rect starting `baseline` above the
//! text baseline), then each formula is drawn with cairo where Pango put its box.

use std::cell::RefCell;

use gtk::prelude::*;
use gtk::subclass::prelude::*;
use gtk::{glib, graphene, pango};
use ratex_types::{Color, DisplayList};

use crate::math;

glib::wrapper! {
    pub struct InlineMath(ObjectSubclass<imp::InlineMath>)
        @extends gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}

impl InlineMath {
    pub fn new(pieces: Vec<(bool, String)>, em_px: f64) -> Self {
        let obj: Self = glib::Object::new();
        *obj.imp().pieces.borrow_mut() = pieces;
        obj.imp().em_px.set(em_px);
        obj
    }
}

struct Formula {
    byte_index: usize,
    list: DisplayList,
    width: f64,
    height: f64,
    baseline: f64,
}

mod imp {
    use super::*;
    use std::cell::Cell;

    #[derive(Default)]
    pub struct InlineMath {
        pub pieces: RefCell<Vec<(bool, String)>>,
        pub em_px: Cell<f64>,
        /// Display lists for the current theme colour.
        cache: RefCell<Option<(Color, Vec<Option<DisplayList>>)>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for InlineMath {
        const NAME: &'static str = "SpikeInlineMath";
        type Type = super::InlineMath;
        type ParentType = gtk::Widget;
    }

    impl ObjectImpl for InlineMath {}

    impl WidgetImpl for InlineMath {
        fn request_mode(&self) -> gtk::SizeRequestMode {
            gtk::SizeRequestMode::HeightForWidth
        }

        fn measure(&self, orientation: gtk::Orientation, for_size: i32) -> (i32, i32, i32, i32) {
            let (layout, _) = self.layout(if orientation == gtk::Orientation::Vertical {
                for_size
            } else {
                -1
            });
            let (_, logical) = layout.pixel_extents();
            if orientation == gtk::Orientation::Horizontal {
                // Minimum: the widest unbreakable thing; approximate with 1/4.
                (logical.width() / 4, logical.width(), -1, -1)
            } else {
                let baseline = layout.baseline() / pango::SCALE;
                (logical.height(), logical.height(), baseline, baseline)
            }
        }

        // No css_changed override needed: `lists()` re-lays out when the
        // theme colour differs from the cached one, and GTK redraws a widget
        // whose CSS colour changed (gtk_widget_css_changed, AFFECTS_REDRAW).
        fn snapshot(&self, snapshot: &gtk::Snapshot) {
            let obj = self.obj();
            let (layout, formulas) = self.layout(obj.width());
            let fg = obj.color();
            snapshot.append_layout(&layout, &fg);
            let font_size = self.em_px.get() * crate::MATH_SCALE;
            for f in formulas {
                let pos = layout.index_to_pos(f.byte_index as i32);
                let x = pos.x() as f32 / pango::SCALE as f32;
                let y = pos.y() as f32 / pango::SCALE as f32;
                snapshot.save();
                snapshot.translate(&graphene::Point::new(x, y));
                let cr = snapshot.append_cairo(&graphene::Rect::new(
                    0.0,
                    0.0,
                    f.width as f32,
                    f.height as f32,
                ));
                let _ = ratex_cairo::render_to_cairo(&cr, &f.list, &math::options(font_size, 0.0));
                snapshot.restore();
                let _ = f.baseline;
            }
        }
    }

    impl InlineMath {
        fn lists(&self) -> Vec<Option<DisplayList>> {
            let color = math::color(&self.obj().color());
            if let Some((c, lists)) = &*self.cache.borrow() {
                if *c == color {
                    return lists.clone();
                }
            }
            let lists: Vec<_> = self
                .pieces
                .borrow()
                .iter()
                .map(|(is_math, s)| {
                    is_math
                        .then(|| math::display_list(s, false, color).ok())
                        .flatten()
                })
                .collect();
            self.cache.replace(Some((color, lists.clone())));
            lists
        }

        pub(super) fn layout(&self, width: i32) -> (pango::Layout, Vec<Formula>) {
            let obj = self.obj();
            let font_size = self.em_px.get() * crate::MATH_SCALE;
            let lists = self.lists();
            let mut text = String::new();
            let attrs = pango::AttrList::new();
            let mut formulas = Vec::new();
            for ((is_math, s), list) in self.pieces.borrow().iter().zip(lists) {
                match (is_math, list) {
                    (false, _) => text.push_str(s),
                    (true, None) => {
                        // Invalid LaTeX: the source in monospace (SPEC fallback).
                        let start = text.len();
                        text.push_str(s);
                        let mut a = pango::AttrString::new_family("monospace");
                        a.set_start_index(start as u32);
                        a.set_end_index(text.len() as u32);
                        attrs.insert(a);
                    }
                    (true, Some(list)) => {
                        let m = math::metrics(&list, font_size, 0.0);
                        let start = text.len();
                        text.push('\u{FFFC}');
                        let s = f64::from(pango::SCALE);
                        let rect = pango::Rectangle::new(
                            0,
                            (-m.baseline * s).round() as i32,
                            (m.width * s).round() as i32,
                            (m.total_height * s).round() as i32,
                        );
                        let mut a = pango::AttrShape::new(&rect, &rect);
                        a.set_start_index(start as u32);
                        a.set_end_index(text.len() as u32);
                        attrs.insert(a);
                        formulas.push(Formula {
                            byte_index: start,
                            list,
                            width: m.width,
                            height: m.total_height,
                            baseline: m.baseline,
                        });
                    }
                }
            }
            let layout = obj.create_pango_layout(Some(&text));
            layout.set_attributes(Some(&attrs));
            layout.set_wrap(pango::WrapMode::WordChar);
            if width > 0 {
                layout.set_width(width * pango::SCALE);
            }
            (layout, formulas)
        }
    }
}
