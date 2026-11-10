//! `MathLine`: a card side's one-line summary in a list row, with its
//! formulas drawn ("σ", not "\sigma"; issue #128, ADR 0077).
//!
//! One Pango layout holds the text, with the object replacement character
//! where each formula goes and a shape attribute (`AttrShape`) the size of
//! the formula, as in [`super::math_paragraph`]. Unlike a paragraph, the
//! line never wraps: Pango ellipsises it at the end, like a `GtkLabel` with
//! `ellipsize: end`. A formula Pango ellipsised away is not drawn: Pango
//! replaces the cut text with one run whose item covers it and carries
//! `PANGO_ANALYSIS_FLAG_IS_ELLIPSIS` (pango/ellipsize.c, `shape_ellipsis`),
//! so every formula placed inside that run is hidden.
//!
//! The formulas are drawn by the widget itself into cairo nodes, as
//! `MathView` draws one, and not as child widgets: a recycled list row
//! rebinds often, and a bind then only replaces a string and a few sources.
//! A formula is drawn at the line's font size × [`MATH_SCALE`], as
//! everywhere else, but never taller than [`MAX_LINE_EM`] body ems (a
//! matrix is shrunk to fit), so rows keep about one line's height. One
//! wider than the whole line is cut at its end, followed by "…". Invalid
//! LaTeX, a formula over a limit (`math`), and formulas past
//! [`MAX_LINE_FORMULAS`] are their source, as plain text.
//!
//! Its CSS name is `label`, so libadwaita's row rules for labels
//! (`row label.subtitle`: smaller and dimmed) style it as the label it
//! replaces. Card text is untrusted: the layout gets plain text (never
//! markup), and the accessible label is the summary's plain text, formulas
//! as their source. Each snapshot marks its area grown by
//! `ink_box::INK_MARGIN` for repainting (ADR 0076), so a recycled row
//! leaves no stray pixels of the card it showed before.

use gtk::glib;
use gtk::subclass::prelude::*;
use rappel_core::content::{Summary, SummaryPiece};

use super::layout_key::{LAYOUT_CACHE_ENTRIES, LayoutKey as Fingerprint, PLACEHOLDER_BYTES};
use super::markup::PLACEHOLDER;
use super::math::MATH_SCALE;

/// The tallest a formula is drawn in a row, in ems of the line's font: a
/// fraction fits; a matrix is shrunk to this.
pub const MAX_LINE_EM: f64 = 1.6;

/// At most this many formulas are drawn in one line; later ones are their
/// source. A summary is at most 200 characters, so this is only reached by
/// lines of tiny formulas, far past what a row shows.
pub const MAX_LINE_FORMULAS: usize = 32;

glib::wrapper! {
    pub struct MathLine(ObjectSubclass<imp::MathLine>)
        @extends gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}

impl Default for MathLine {
    fn default() -> Self {
        glib::Object::new()
    }
}

/// What a line shows: its text, with a [`PLACEHOLDER`] for each formula,
/// and the formulas' sources with where their placeholders are.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LineContent {
    pub text: String,
    pub formulas: Vec<(usize, String)>,
}

/// The [`LineContent`] of `summary`: each formula `drawable` accepts is a
/// placeholder, up to [`MAX_LINE_FORMULAS`]; any other is its source.
pub fn line_content(summary: &Summary, drawable: impl Fn(&str) -> bool) -> LineContent {
    let mut out = LineContent::default();
    for piece in &summary.pieces {
        match piece {
            SummaryPiece::Text(text) => out.text.push_str(text),
            SummaryPiece::Math(source) => {
                if out.formulas.len() < MAX_LINE_FORMULAS && drawable(source) {
                    out.formulas.push((out.text.len(), source.clone()));
                    out.text.push(PLACEHOLDER);
                } else {
                    // One line, as the summary's text has it.
                    let mut words = source.split_whitespace();
                    if let Some(first) = words.next() {
                        out.text.push_str(first);
                    }
                    for word in words {
                        out.text.push(' ');
                        out.text.push_str(word);
                    }
                }
            }
        }
    }
    out
}

/// The scale at which a formula `height` pixels tall (at the line's math
/// size) is drawn so it is at most `max` pixels tall: 1, or less.
pub fn fit_scale(height: f64, max: f64) -> f64 {
    if height > max && height > 0.0 && max > 0.0 {
        max / height
    } else {
        1.0
    }
}

/// The width of the box Pango reserves for a formula `width` pixels wide in
/// a line `line` pixels wide (-1: no limit), leaving room for an ellipsis
/// `ellipsis` pixels wide after a formula that has to be cut.
pub fn line_box_width(width: f64, line: i32, ellipsis: f64) -> f64 {
    if line > 0 {
        width.min((f64::from(line) - ellipsis).max(0.0))
    } else {
        width
    }
}

impl MathLine {
    /// Shows `summary`. Cheap when it is what the line shows already.
    pub fn set_summary(&self, summary: &Summary) {
        use gtk::prelude::WidgetExt;
        let rgba = self.color();
        let color = ratex_types::Color::new(rgba.red(), rgba.green(), rgba.blue(), rgba.alpha());
        let content = line_content(summary, |source| {
            // In the colour it is drawn in, so the check's layout is the
            // one drawn, from the cache (`math::formula`).
            super::math::formula(source, false, color).is_ok()
        });
        self.imp().set_content(content, &summary.text);
    }

    /// The text laid out, with a placeholder for each formula drawn.
    #[cfg(test)]
    pub fn content(&self) -> LineContent {
        self.imp().content.borrow().clone()
    }

    /// Which formulas are drawn when the line is `width` pixels wide (-1:
    /// no limit), and how wide each one's box is, in pixels.
    #[cfg(test)]
    pub fn drawn_at(&self, width: i32) -> Vec<(String, bool, f64)> {
        self.imp()
            .laid(width)
            .formulas
            .iter()
            .map(|f| (f.source.clone(), f.shown, f.box_width))
            .collect()
    }
}

/// One formula as laid out at one width.
#[derive(Debug, Clone)]
struct Placed {
    /// Its place in the line's content (`LineContent::formulas`), which
    /// is also its slot in the display-list memo.
    slot: usize,
    index: usize,
    source: String,
    /// The font size it is drawn at, in pixels.
    font_size: f64,
    /// Its drawn size, in pixels.
    width: f64,
    height: f64,
    /// The part above the baseline, in pixels.
    ascent: f64,
    /// The width Pango reserved for it, in pixels.
    box_width: f64,
    /// Not ellipsised away.
    shown: bool,
}

/// A layout at one width, with its formulas.
#[derive(Debug)]
struct Laid {
    fingerprint: Fingerprint,
    layout: gtk::pango::Layout,
    formulas: Vec<Placed>,
    /// "…" in the line's font, drawn after a formula cut at the line's
    /// end, and its width in pixels.
    dots: gtk::pango::Layout,
    ellipsis: f64,
}

mod imp {
    use std::cell::RefCell;
    use std::panic::{AssertUnwindSafe, catch_unwind};
    use std::rc::Rc;

    use gtk::prelude::*;
    use gtk::subclass::prelude::*;
    use gtk::{glib, graphene, pango};
    use ratex_cairo::{CairoOptions, render_to_cairo};
    use ratex_types::{Color, DisplayList};

    use super::super::ink_box::mark_damage;
    use super::super::math::{self, ceil_px, f32_px, from_pango_units, left_edge, to_pango_units};
    use super::super::math_view::em_px;
    use super::{
        Fingerprint, LAYOUT_CACHE_ENTRIES, Laid, LineContent, MATH_SCALE, MAX_LINE_EM,
        PLACEHOLDER_BYTES, Placed, fit_scale, line_box_width,
    };

    /// A formula's display list and the colour it was made for.
    type Memo = (Color, Rc<DisplayList>);

    #[derive(Debug, Default)]
    pub struct MathLine {
        pub(super) content: RefCell<LineContent>,
        /// Each formula's display list in the colour it was made for, so
        /// measuring and drawing do not look it up again.
        memo: RefCell<Vec<Option<Memo>>>,
        cache: RefCell<Vec<Rc<Laid>>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for MathLine {
        const NAME: &'static str = "RappelMathLine";
        type Type = super::MathLine;
        type ParentType = gtk::Widget;

        fn class_init(klass: &mut Self::Class) {
            // libadwaita's row rules name labels (`row label.subtitle`).
            klass.set_css_name("label");
            klass.set_accessible_role(gtk::AccessibleRole::Label);
        }
    }

    impl ObjectImpl for MathLine {}

    impl WidgetImpl for MathLine {
        fn request_mode(&self) -> gtk::SizeRequestMode {
            gtk::SizeRequestMode::ConstantSize
        }

        fn measure(&self, orientation: gtk::Orientation, _for_size: i32) -> (i32, i32, i32, i32) {
            let full = self.laid(-1);
            let (_, logical) = full.layout.pixel_extents();
            if orientation == gtk::Orientation::Horizontal {
                let natural = logical.width();
                // Like an ellipsising label: down to "…".
                (natural.min(ceil_px(full.ellipsis)), natural, -1, -1)
            } else {
                let baseline = full.layout.baseline() / pango::SCALE;
                (logical.height(), logical.height(), baseline, baseline)
            }
        }

        fn snapshot(&self, snapshot: &gtk::Snapshot) {
            let obj = self.obj();
            let (width, height) = (obj.width(), obj.height());
            if width <= 0 || height <= 0 {
                return;
            }
            mark_damage(
                snapshot,
                &graphene::Rect::new(
                    0.0,
                    0.0,
                    f32_px(f64::from(width)),
                    f32_px(f64::from(height)),
                ),
            );
            let laid = self.laid(width);
            // The line keeps the baseline it was measured with, even when
            // the formula that made it tall was ellipsised away.
            let full = self.laid(-1);
            // Centred in extra height (a CSS `min-height`), as `GtkLabel`
            // centres its text, so a label beside it lines up.
            let (_, logical) = full.layout.pixel_extents();
            let top = f64::from((height - logical.height()).max(0)) / 2.0;
            let shift = top + from_pango_units(full.layout.baseline() - laid.layout.baseline());
            let baseline = top + from_pango_units(full.layout.baseline());
            let color = obj.color();
            snapshot.save();
            snapshot.translate(&graphene::Point::new(0.0, f32_px(shift)));
            snapshot.append_layout(&laid.layout, &color);
            snapshot.restore();
            let draw_color = Color::new(color.red(), color.green(), color.blue(), color.alpha());
            for placed in &laid.formulas {
                if !placed.shown {
                    continue;
                }
                let Some(list) = self.list(placed.slot, &placed.source, draw_color) else {
                    continue;
                };
                let pos = laid
                    .layout
                    .index_to_pos(i32::try_from(placed.index).unwrap_or(i32::MAX));
                let left = from_pango_units(left_edge(pos.x(), pos.width()));
                let top = baseline - placed.ascent;
                let cut = placed.width > placed.box_width + 0.5;
                snapshot.save();
                snapshot.translate(&graphene::Point::new(f32_px(left), f32_px(top)));
                if cut {
                    snapshot.push_clip(&graphene::Rect::new(
                        0.0,
                        0.0,
                        f32_px(placed.box_width),
                        f32_px(placed.height),
                    ));
                }
                let cr = snapshot.append_cairo(&graphene::Rect::new(
                    0.0,
                    0.0,
                    f32_px(placed.width.ceil()),
                    f32_px(placed.height.ceil()),
                ));
                let options = CairoOptions {
                    font_size: placed.font_size,
                    padding: 0.0,
                    font_dir: None,
                };
                // As in `MathView`: a bug in the drawing code must not take
                // the window down.
                match catch_unwind(AssertUnwindSafe(|| render_to_cairo(&cr, &list, &options))) {
                    Ok(Ok(())) => {}
                    Ok(Err(error)) => tracing::debug!(%error, "could not draw a formula"),
                    Err(_) => tracing::warn!("drawing a formula panicked"),
                }
                drop(cr);
                if cut {
                    snapshot.pop();
                }
                snapshot.restore();
                // A formula wider than the line ends in "…", unless Pango
                // put one after it already.
                if cut && !laid.layout.is_ellipsized() {
                    let dots = &laid.dots;
                    let x = if pos.width() < 0 {
                        left - laid.ellipsis
                    } else {
                        left + placed.box_width
                    };
                    let y = baseline - from_pango_units(dots.baseline());
                    snapshot.save();
                    snapshot.translate(&graphene::Point::new(f32_px(x), f32_px(y)));
                    snapshot.append_layout(dots, &color);
                    snapshot.restore();
                }
            }
        }
    }

    impl MathLine {
        pub(super) fn set_content(&self, content: LineContent, spoken: &str) {
            if *self.content.borrow() == content {
                return;
            }
            let n = content.formulas.len();
            self.content.replace(content);
            self.memo.replace(vec![None; n]);
            self.cache.borrow_mut().clear();
            let obj = self.obj();
            obj.update_property(&[gtk::accessible::Property::Label(spoken)]);
            obj.queue_resize();
        }

        /// Formula `i`'s display list in `color`.
        fn list(&self, i: usize, source: &str, color: Color) -> Option<Rc<DisplayList>> {
            if let Some(Some((made_for, list))) = self.memo.borrow().get(i)
                && *made_for == color
            {
                return Some(Rc::clone(list));
            }
            let list = math::formula(source, false, color).ok()?;
            if let Some(slot) = self.memo.borrow_mut().get_mut(i) {
                *slot = Some((color, Rc::clone(&list)));
            }
            Some(list)
        }

        /// The layout for `width` pixels (-1: no limit), made again only
        /// when something it depends on changed.
        pub(super) fn laid(&self, width: i32) -> Rc<Laid> {
            let fingerprint = Fingerprint::of(&*self.obj(), width);
            if let Some(found) = self
                .cache
                .borrow()
                .iter()
                .find(|laid| laid.fingerprint == fingerprint)
            {
                return Rc::clone(found);
            }
            let laid = Rc::new(self.lay_out(fingerprint));
            let mut cache = self.cache.borrow_mut();
            if cache.len() >= LAYOUT_CACHE_ENTRIES {
                cache.remove(0);
            }
            cache.push(Rc::clone(&laid));
            laid
        }

        fn lay_out(&self, fingerprint: Fingerprint) -> Laid {
            let obj = self.obj();
            let width = fingerprint.width();
            let content = self.content.borrow();
            let layout = obj.create_pango_layout(Some(&content.text));
            layout.set_single_paragraph_mode(true);
            layout.set_ellipsize(pango::EllipsizeMode::End);
            if width > 0 {
                layout.set_width(width.saturating_mul(pango::SCALE));
            }
            let dots = obj.create_pango_layout(Some("…"));
            let ellipsis = from_pango_units(dots.size().0);
            let em = em_px(&*obj);
            let attrs = pango::AttrList::new();
            let color = obj.color();
            let color = Color::new(color.red(), color.green(), color.blue(), color.alpha());
            let mut formulas = Vec::with_capacity(content.formulas.len());
            for (i, (index, source)) in content.formulas.iter().enumerate() {
                let Some(list) = self.list(i, source, color) else {
                    continue;
                };
                let math_size = em * MATH_SCALE;
                let scale = fit_scale((list.height + list.depth) * math_size, MAX_LINE_EM * em);
                let font_size = math_size * scale;
                let placed_width = list.width * font_size;
                let height = (list.height + list.depth) * font_size;
                let ascent = list.height * font_size;
                let box_width = line_box_width(placed_width, width, ellipsis);
                let rect = pango::Rectangle::new(
                    0,
                    to_pango_units(-ascent),
                    to_pango_units(box_width),
                    to_pango_units(height),
                );
                let mut shape = pango::AttrShape::new(&rect, &rect);
                let start = u32::try_from(*index).unwrap_or(u32::MAX);
                shape.set_start_index(start);
                shape.set_end_index(start.saturating_add(PLACEHOLDER_BYTES));
                attrs.insert(shape);
                formulas.push(Placed {
                    slot: i,
                    index: *index,
                    source: source.clone(),
                    font_size,
                    width: placed_width,
                    height,
                    ascent,
                    box_width,
                    shown: true,
                });
            }
            layout.set_attributes(Some(&attrs));
            if let Some(gap) = ellipsised_range(&layout) {
                for placed in &mut formulas {
                    placed.shown = !gap.contains(&placed.index);
                }
            }
            Laid {
                fingerprint,
                layout,
                formulas,
                dots,
                ellipsis,
            }
        }
    }

    /// The bytes of the layout's text that Pango ellipsised away, if any:
    /// the item of its ellipsis run (pango/ellipsize.c, `shape_ellipsis`).
    fn ellipsised_range(layout: &pango::Layout) -> Option<std::ops::Range<usize>> {
        if !layout.is_ellipsized() {
            return None;
        }
        let line = layout.line_readonly(0)?;
        line.runs().into_iter().find_map(|run| {
            let item = run.item();
            let analysis = item.analysis();
            (i32::from(analysis.flags()) & pango::ANALYSIS_FLAG_IS_ELLIPSIS != 0).then(|| {
                let start = usize::try_from(item.offset()).unwrap_or(usize::MAX);
                let length = usize::try_from(item.length()).unwrap_or(0);
                start..start.saturating_add(length)
            })
        })
    }
}

#[cfg(test)]
mod tests {
    use gtk::prelude::*;
    use rappel_core::content::summary;

    use super::*;

    #[test]
    fn formulas_become_placeholders_and_the_rest_their_source() {
        let content = line_content(&summary("Mesure $\\sigma$-finie $x^$"), |s| s == "\\sigma");
        assert_eq!(content.text, "Mesure \u{FFFC}-finie x^");
        assert_eq!(content.formulas, [(7, "\\sigma".to_owned())]);
        // A source on several lines is one line of text.
        let content = line_content(&summary("$$a\n= b$$"), |_| false);
        assert_eq!(content.text, "a = b");
    }

    #[test]
    fn only_so_many_formulas_are_drawn_per_line() {
        let md = "$x$ ".repeat(MAX_LINE_FORMULAS + 5);
        let content = line_content(&summary(&md), |_| true);
        assert_eq!(content.formulas.len(), MAX_LINE_FORMULAS);
        assert!(content.text.ends_with('x'));
    }

    #[test]
    fn a_tall_formula_is_shrunk_to_the_line_and_a_short_one_is_not() {
        assert_eq!(fit_scale(10.0, 24.0), 1.0);
        assert!((fit_scale(48.0, 24.0) - 0.5).abs() < 1e-9);
        assert_eq!(fit_scale(f64::NAN, 24.0), 1.0);
        assert_eq!(fit_scale(10.0, 0.0), 1.0);
    }

    #[test]
    fn a_formula_wider_than_the_line_leaves_room_for_the_ellipsis() {
        assert_eq!(line_box_width(50.0, -1, 10.0), 50.0);
        assert_eq!(line_box_width(50.0, 100, 10.0), 50.0);
        assert_eq!(line_box_width(500.0, 100, 10.0), 90.0);
        assert_eq!(line_box_width(500.0, 5, 10.0), 0.0);
    }

    /// Needs a display; CI runs the tests under Xvfb (ADR 0024).
    #[test]
    fn a_line_draws_its_formulas_and_hides_those_ellipsised_away() {
        crate::testing::with_widgets(|| {
            let line = MathLine::default();
            line.set_summary(&summary("Mesure $\\sigma$-finie"));
            assert_eq!(line.content().formulas.len(), 1);
            let full = line.drawn_at(-1);
            assert_eq!(full.len(), 1);
            assert!(full[0].1, "shown at full width");
            let (_, natural, _, _) = line.measure(gtk::Orientation::Horizontal, -1);
            // Wide enough: drawn. Too narrow for "Mesure ": ellipsised away.
            assert!(line.drawn_at(natural + 10)[0].1);
            assert!(!line.drawn_at(12)[0].1);
            // Broken LaTeX is its source, as text.
            line.set_summary(&summary("Mesure $x^$"));
            assert_eq!(line.content().formulas, []);
            assert_eq!(line.content().text, "Mesure x^");
            // A formula alone, wider than the line, keeps a box that fits
            // with the "…" after it.
            line.set_summary(&summary(
                "$\\sum_{i=1}^{n} i^2 = \\frac{n(n+1)(2n+1)}{6} + \\int_0^1 x^2 \\, dx$",
            ));
            let narrow = line.drawn_at(60);
            assert!(narrow[0].1, "a lone formula is drawn, cut");
            assert!(narrow[0].2 < 60.0);
        });
    }
}
