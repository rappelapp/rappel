//! `InkBox`: the box that holds a rendered card side, whose redraws cover
//! every pixel its text can draw (issue #120, ADR 0076).
//!
//! GTK repaints only the damage: the parts of the window whose render
//! nodes changed since the last frame (`gsk_render_node_diff`, rounded out
//! to whole pixels by `rectangle_init_from_graphene`, gsk/gskrendernode.c).
//! A text node's bounds are the glyphs' ink extents *without hinting*
//! (`gsk_get_glyph_string_extents`, gsk/gskprivate.c: "like
//! `pango_glyph_string_extents`, but it ignores hinting of the font"), but
//! the glyphs are drawn hinted: GTK gives every widget's Pango context
//! `CAIRO_HINT_STYLE_SLIGHT` unless the font rendering is set to manual
//! (`gtk_widget_update_pango_context`, gtk/gtkwidget.c), and slight hinting
//! moves outlines vertically to the pixel grid. An accent or the top of a
//! tall letter can so be drawn a pixel row above its node's bounds. When
//! the next card replaces that text, the damage is the old and new bounds,
//! and the row outside them keeps the old card's pixels: the light dots
//! above "Tribu engendrée". When text appears, the same row is missing
//! from the new card until something else repaints it.
//!
//! The box adds, under its children, a transparent colour node over their
//! bounds grown by [`INK_MARGIN`] ([`damage_bounds`]), with a colour that
//! is new at every snapshot ([`marker_color`]). A colour node whose colour
//! or bounds changed is damaged as a whole (`gsk_color_node_diff`,
//! gsk/gskcolornode.c), so whenever the side draws anew, the whole area its
//! glyphs can reach is repainted. GTK calls the snapshot again only when
//! something in the side asked to be drawn (new content, the reveal's
//! slide, a hovered link or button, a playing sound's time when its text
//! changes: once a second), and keeps the box's node otherwise, so the
//! frames in between repaint no more than before.

use gtk::glib;
use gtk::{gdk, graphene};

/// How far, in logical pixels, a drawn glyph may reach beyond the bounds
/// GTK gives its text node. Hinting moves an outline by less than a device
/// pixel, so its antialiased edge reaches at most one pixel row or column
/// past the node's bounds rounded out: the test below measured one, never
/// two (ADR 0076). A device pixel is at most a logical one (scale ≥ 1), so
/// one logical pixel would be enough; two leave the same again for a font
/// or renderer that rounds differently.
pub const INK_MARGIN: f32 = 2.0;

/// The area a side's drawing can touch: `drawn` (the bounds of its render
/// node) grown by `margin` on every side.
pub fn damage_bounds(drawn: &graphene::Rect, margin: f32) -> graphene::Rect {
    graphene::Rect::new(
        drawn.x() - margin,
        drawn.y() - margin,
        drawn.width() + 2.0 * margin,
        drawn.height() + 2.0 * margin,
    )
}

/// The marker's colour for the `n`th snapshot: fully transparent, so it
/// never shows, and different for each `n` below 2²⁴ (its low 24 bits are
/// the red, green and blue), so two snapshots in a row never compare equal.
pub fn marker_color(n: u32) -> gdk::RGBA {
    let channel = |shift: u32| f32::from(((n >> shift) & 0xFF) as u8) / 255.0;
    gdk::RGBA::new(channel(16), channel(8), channel(0), 0.0)
}

thread_local! {
    /// Shared by every box and row line, so a side that replaces another (a
    /// new widget in the same place) never draws the same marker either.
    static SNAPSHOTS: std::cell::Cell<u32> = const { std::cell::Cell::new(0) };
}

/// Appends to `snapshot` the invisible marker over `drawn` grown by
/// [`INK_MARGIN`], in a colour new at each call: whatever is drawn after it
/// in `drawn` is then repainted with a margin wide enough for hinted glyphs.
/// `InkBox` does it for a card side; a card list's row lines
/// (`render::MathLine`) do it for themselves.
pub fn mark_damage(snapshot: &gtk::Snapshot, drawn: &graphene::Rect) {
    use gtk::prelude::SnapshotExt;

    let n = SNAPSHOTS.with(|count| {
        let n = count.get().wrapping_add(1);
        count.set(n);
        n
    });
    // GTK asks not to append transparent colours "if you can avoid it"
    // (gtk_snapshot_append_color docs), for the drawing cost; this one is
    // drawn as nothing and is here for the damage.
    snapshot.append_color(&marker_color(n), &damage_bounds(drawn, INK_MARGIN));
}

glib::wrapper! {
    pub struct InkBox(ObjectSubclass<imp::InkBox>)
        @extends gtk::Box, gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget, gtk::Orientable;
}

impl InkBox {
    /// A vertical box with `spacing` between its children.
    pub fn new(spacing: i32) -> Self {
        glib::Object::builder()
            .property("orientation", gtk::Orientation::Vertical)
            .property("spacing", spacing)
            .build()
    }
}

mod imp {
    use gtk::glib;
    use gtk::prelude::*;
    use gtk::subclass::prelude::*;

    use super::mark_damage;

    #[derive(Debug, Default)]
    pub struct InkBox;

    #[glib::object_subclass]
    impl ObjectSubclass for InkBox {
        const NAME: &'static str = "RappelInkBox";
        type Type = super::InkBox;
        type ParentType = gtk::Box;
    }

    impl ObjectImpl for InkBox {}

    impl WidgetImpl for InkBox {
        fn snapshot(&self, snapshot: &gtk::Snapshot) {
            // The children are drawn first into a snapshot of their own
            // (at the same origin: a new snapshot starts with no
            // transform), only to learn their bounds.
            let children = gtk::Snapshot::new();
            self.parent_snapshot(&children);
            let Some(node) = children.to_node() else {
                return;
            };
            mark_damage(snapshot, &node.bounds());
            snapshot.append_node(&node);
        }
    }

    impl BoxImpl for InkBox {}
}

#[cfg(test)]
mod tests {
    use gtk::prelude::*;
    use gtk::{gdk, graphene, gsk, pango};

    use super::*;

    #[test]
    fn the_damage_is_the_drawn_area_grown_on_every_side() {
        let drawn = graphene::Rect::new(10.0, 20.5, 100.0, 30.0);
        let grown = damage_bounds(&drawn, 2.0);
        assert_eq!(
            (grown.x(), grown.y(), grown.width(), grown.height()),
            (8.0, 18.5, 104.0, 34.0)
        );
        assert!(grown.contains_rect(&drawn));
    }

    #[test]
    fn the_marker_is_invisible_and_new_at_each_snapshot() {
        let colors: Vec<_> = [0, 1, 2, 255, 256, 65_536, 0xFF_FFFF]
            .into_iter()
            .map(marker_color)
            .collect();
        for (i, a) in colors.iter().enumerate() {
            assert_eq!(a.alpha(), 0.0);
            for b in &colors[i + 1..] {
                assert_ne!(a, b);
            }
        }
        // Two snapshots in a row differ even where the counter wraps.
        assert_ne!(
            marker_color(u32::MAX),
            marker_color(u32::MAX.wrapping_add(1))
        );
    }

    /// The box's own snapshot, as GTK takes it while drawing the window.
    fn snapshot_of(ink: &InkBox) -> gsk::RenderNode {
        use gtk::subclass::prelude::*;
        let snapshot = gtk::Snapshot::new();
        WidgetImpl::snapshot(ink.imp(), &snapshot);
        snapshot.to_node().expect("a box with text draws")
    }

    /// The marker of a snapshot: its first node, a colour node. The
    /// children's nodes follow it (a container appended to a snapshot is
    /// spliced into it, `gtk_snapshot_append_node_internal`).
    fn marker_of(node: &gsk::RenderNode) -> gsk::ColorNode {
        let container = node
            .downcast_ref::<gsk::ContainerNode>()
            .expect("the marker and the children");
        assert!(container.n_children() >= 2, "a marker and something drawn");
        container
            .child(0)
            .downcast::<gsk::ColorNode>()
            .expect("the marker comes first")
    }

    /// A rendered side is an `InkBox`, and each of its snapshots puts an
    /// invisible colour node of a new colour under its children, over
    /// their bounds grown by the margin.
    /// Needs a display; CI runs the tests under Xvfb (ADR 0024).
    #[test]
    fn a_side_marks_its_whole_ink_at_every_snapshot() {
        let (is_ink_box, first, second, children) = crate::testing::with_gtk(|| {
            let side = crate::render::render(
                &rappel_core::content::parse("Tribu engendrée *σ-algèbre*"),
                &crate::render::Images::default(),
            );
            let is_ink_box = side.is::<InkBox>();
            let ink = side.downcast::<InkBox>().expect("checked above");
            let window = gtk::Window::builder().child(&ink).build();
            window.present();
            // Until every shown child has its size: a window on a compositor (an
            // X server with a window manager, such as Xwayland in a desktop
            // session) is mapped and allocated only once the compositor
            // answers, later than one turn of the main loop; without a size a
            // label draws nothing ("Trying to snapshot GtkLabel … without a
            // current allocation"), and the box would have nothing to mark.
            // Under Xvfb, with no window manager, the first turn does it.
            let laid_out = crate::testing::settle_until(
                || {
                    crate::testing::descendants::<gtk::Widget>(ink.upcast_ref())
                        .iter()
                        .filter(|child| child.is_visible())
                        .all(|child| child.width() > 0 && child.height() > 0)
                },
                std::time::Duration::from_secs(10),
            );
            assert!(laid_out, "the side is laid out");
            let (first, second) = (snapshot_of(&ink), snapshot_of(&ink));
            window.destroy();
            let children = first.downcast_ref::<gsk::ContainerNode>().map(|c| {
                (2..c.n_children()).fold(c.child(1).bounds(), |all, i| {
                    all.union(&c.child(i).bounds())
                })
            });
            let marker = |node: &gsk::RenderNode| {
                let marker = marker_of(node);
                (marker.color(), marker.bounds())
            };
            (is_ink_box, marker(&first), marker(&second), children)
        });
        assert!(is_ink_box, "render returns an InkBox");
        let children = children.expect("the snapshot is a container");
        let (color, bounds) = first;
        assert_eq!(color.alpha(), 0.0);
        let grown = damage_bounds(&children, INK_MARGIN);
        assert_eq!(
            (bounds.x(), bounds.y(), bounds.width(), bounds.height()),
            (grown.x(), grown.y(), grown.width(), grown.height())
        );
        assert_ne!(color, second.0, "two snapshots in a row differ");
        assert_eq!(second.0.alpha(), 0.0);
    }

    /// Accents above capitals (stacked on "Ǻ"), accents on lower case,
    /// italics that lean out of their box, and the issue's own text.
    const TEXT: &str = "Ǻǻ Ŝŷ ĝĥ ÉÂÎ Tribu engendrée <i>σ-algèbre ƒ fjord</i> Ḟ ŰŐ";

    /// The pixel rows and columns that `node` paints, as a rectangle in
    /// its coordinates, found by drawing it with GTK's cairo renderer
    /// (the one the screenshots reproduced the bug with).
    fn painted(node: &gsk::RenderNode) -> graphene::Rect {
        let renderer = gsk::CairoRenderer::new();
        renderer
            .realize_for_display(&gdk::Display::default().expect("GTK has a display"))
            .expect("the cairo renderer always realizes");
        // Far wider than any overhang, in whole pixels.
        let view = damage_bounds(&node.bounds(), 16.0);
        let view = graphene::Rect::new(
            view.x().floor(),
            view.y().floor(),
            view.width().ceil(),
            view.height().ceil(),
        );
        let texture = renderer.render_texture(node, Some(&view));
        renderer.unrealize();
        let mut downloader = gdk::TextureDownloader::new(&texture);
        downloader.set_format(gdk::MemoryFormat::R8g8b8a8);
        let (bytes, stride) = downloader.download_bytes();
        let (width, height) = (
            usize::try_from(texture.width()).expect("a size is positive"),
            usize::try_from(texture.height()).expect("a size is positive"),
        );
        let (mut left, mut top, mut right, mut bottom) = (usize::MAX, usize::MAX, 0, 0);
        for y in 0..height {
            for x in 0..width {
                if bytes[y * stride + x * 4 + 3] > 0 {
                    left = left.min(x);
                    top = top.min(y);
                    right = right.max(x + 1);
                    bottom = bottom.max(y + 1);
                }
            }
        }
        assert!(left < right, "the text paints something");
        let px = |n: usize| f32::from(u16::try_from(n).expect("a small texture"));
        graphene::Rect::new(
            view.x() + px(left),
            view.y() + px(top),
            px(right - left),
            px(bottom - top),
        )
    }

    fn rect(r: &pango::Rectangle) -> graphene::Rect {
        let f = |n: i32| f32::from(i16::try_from(n).expect("a small layout"));
        graphene::Rect::new(f(r.x()), f(r.y()), f(r.width()), f(r.height()))
    }

    /// For text with accents and italics at body sizes, in every font
    /// size from 10 to 40 px: what GTK draws, and Pango's (hinted) ink
    /// extents, both fit in [`damage_bounds`] of the node's bounds. Prints
    /// how many pixel rows and columns the drawing reached past the node's
    /// own bounds rounded out, which is GTK's damage (the bug):
    /// fonts differ between machines, so that is not asserted.
    /// Needs a display; CI runs the tests under Xvfb (ADR 0024).
    #[test]
    fn the_damage_covers_what_text_with_accents_and_italics_draws() {
        let reports = crate::testing::with_gtk(|| {
            // A widget's context: the font options GTK gives every widget
            // (slight hinting), as a card's labels get them.
            let widget = gtk::Label::new(None);
            let mut reports = Vec::new();
            for size in 10..=40 {
                let layout = widget.create_pango_layout(None);
                layout.set_markup(TEXT);
                let mut font = pango::FontDescription::from_string("Sans");
                font.set_absolute_size(f64::from(size * pango::SCALE));
                layout.set_font_description(Some(&font));
                // At a fractional offset as well as a whole one: a line's
                // baseline is not always on a pixel.
                for offset in [0.0, 0.25, 0.5, 0.75] {
                    let snapshot = gtk::Snapshot::new();
                    snapshot.translate(&graphene::Point::new(offset, offset));
                    snapshot.append_layout(&layout, &gdk::RGBA::WHITE);
                    let node = snapshot.to_node().expect("the text draws a node");
                    let bounds = node.bounds();
                    let damage = damage_bounds(&bounds, INK_MARGIN);
                    let drawn = painted(&node);
                    let (ink, _) = layout.pixel_extents();
                    let mut ink = rect(&ink);
                    ink.offset(offset, offset);
                    assert!(
                        damage.contains_rect(&drawn),
                        "{size} px at +{offset}: drawn {drawn:?} outside {damage:?}"
                    );
                    assert!(
                        damage.contains_rect(&ink),
                        "{size} px at +{offset}: Pango's ink {ink:?} outside {damage:?}"
                    );
                    // GTK's damage: the bounds rounded out to pixels.
                    let (top, left) = (bounds.y().floor(), bounds.x().floor());
                    let bottom = (bounds.y() + bounds.height()).ceil();
                    let right = (bounds.x() + bounds.width()).ceil();
                    reports.push((
                        size,
                        offset,
                        top - drawn.y(),
                        drawn.y() + drawn.height() - bottom,
                        left - drawn.x(),
                        drawn.x() + drawn.width() - right,
                    ));
                }
            }
            reports
        });
        let beyond = reports
            .iter()
            .filter(|(.., top, bottom, left, right)| top.max(*bottom).max(*left).max(*right) > 0.0)
            .count();
        // The evidence for ADR 0076, shown with --nocapture.
        {
            eprintln!(
                "{beyond} of {} drawings reach past their text node's bounds",
                reports.len()
            );
            for (size, offset, top, bottom, left, right) in &reports {
                eprintln!(
                    "{size} px +{offset}: past top {top}, bottom {bottom}, left {left}, right {right}"
                );
            }
        }
    }
}
