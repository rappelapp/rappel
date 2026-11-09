//! `BarChart`: one bar per day, with horizontal grid lines carrying their
//! values on the right and a date under every seventh day (SPEC §4.7,
//! ADR 0078). Shapes are drawn with Cairo in a `GtkDrawingArea`, texts with
//! Pango in the widget's own font, so they follow the text size.
//!
//! Theme colours only: the bars are the accent colour (`AdwStyleManager`),
//! an upper part (fails) the same accent at 38 % set off by a 2 px gap and
//! its own rounded top, so the two parts differ by shape and not only by
//! colour; everything else is the widget's CSS `color` at lower opacity,
//! so light, dark and accent changes need no colours of ours.
//!
//! The geometry is the pure [`layout`] module; every text comes in the
//! [`ChartModel`], made by the statistics page's pure code.
//!
//! Keyboard, touch and screen readers: the chart is focusable. Left /
//! Right / Home / End select a day, and so does a press anywhere on the
//! chart (the nearest day, so a finger need not hit a 7 px bar). The
//! selected day gets a light rounded highlight; a selection the user makes
//! is announced with its caption and handed to [`BarChart::connect_selected`].
//! The chart's accessible role is Img; its label and description (a summary
//! of the data in words) come from the model.
//!
//! Redrawing: GTK draws again when the size changes, and the widget queues
//! a draw when `AdwStyleManager` reports a new accent colour or dark mode.
//! A change of the CSS `color` queues one through GTK's default
//! `css_changed` handling (see `render/math_view.rs`).

pub mod layout;

use adw::prelude::*;
use gtk::glib;
use gtk::pango;
use gtk::subclass::prelude::*;
use layout::Bar;

use crate::ui::callback::Callback;

/// Called with the day the user selected.
type OnSelected = dyn Fn(usize);

glib::wrapper! {
    pub struct BarChart(ObjectSubclass<imp::BarChart>)
        @extends gtk::DrawingArea, gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}

/// One day of a chart: its bar, and the sentence said when it is selected.
#[derive(Debug, Clone, PartialEq)]
pub struct ChartBar {
    pub bar: Bar,
    pub caption: String,
}

/// Everything one chart shows.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct ChartModel {
    pub bars: Vec<ChartBar>,
    /// The value axis.
    pub ticks: Vec<Tick>,
    /// The date under each day, if it has one (every seventh day).
    pub dates: Vec<Option<String>>,
    /// The accessible label: what the chart shows.
    pub label: String,
    /// The accessible description: the data in words.
    pub summary: String,
}

impl ChartModel {
    /// The top of the value axis, at least 1.
    pub fn ceiling(&self) -> f64 {
        self.ticks.last().map_or(1.0, |t| t.value).max(1.0)
    }
}

/// A grid line and the value written at its right.
#[derive(Debug, Clone, PartialEq)]
pub struct Tick {
    pub value: f64,
    pub label: String,
}

/// The height of the bars' area in pixels; the width is whatever there is.
const PLOT_HEIGHT: f64 = 150.0;
/// Space between the bars and their value labels, and above the dates.
const LABEL_GAP: f64 = 6.0;
/// Opacities of the widget's text colour: libadwaita's `.dimmed` for the
/// texts, then the grid lines, the base line and the selected day.
const TEXT_OPACITY: f64 = 0.55;
const GRID_OPACITY: f64 = 0.12;
const BASE_OPACITY: f64 = 0.3;
const HIGHLIGHT_OPACITY: f64 = 0.1;
/// The accent's opacity for an upper part (fails), as the brief asks.
const UPPER_OPACITY: f64 = 0.38;
/// The radius of the selected day's highlight.
const HIGHLIGHT_RADIUS: f64 = 6.0;
/// The labels' size relative to the body text: libadwaita's `.caption`.
const LABEL_SCALE: f64 = 0.8333;

impl Default for BarChart {
    fn default() -> Self {
        glib::Object::new()
    }
}

impl BarChart {
    /// Shows `model` with `selected` selected (today, at first), silently:
    /// the page shows the selected day's values itself.
    pub fn set_model(&self, model: ChartModel, selected: Option<usize>) {
        let imp = self.imp();
        self.update_property(&[
            gtk::accessible::Property::Label(&model.label),
            gtk::accessible::Property::Description(&model.summary),
        ]);
        let days = model.bars.len();
        imp.model.replace(model);
        imp.selected.set(selected.filter(|&d| d < days));
        self.set_content_height(imp.height());
        self.queue_draw();
    }

    /// Calls `f` with the day the user selects (keys or a press).
    pub fn connect_selected(&self, f: impl Fn(usize) + 'static) {
        self.imp()
            .on_selected
            .replace(Some(Callback::new(std::rc::Rc::new(f))));
    }

    /// Selects `day`, announces its caption to screen readers and tells
    /// the page. A day the chart does not have is ignored.
    pub fn select(&self, day: usize) {
        let imp = self.imp();
        let Some(caption) = imp.model.borrow().bars.get(day).map(|b| b.caption.clone()) else {
            return;
        };
        imp.selected.set(Some(day));
        self.announce(&caption, gtk::AccessibleAnnouncementPriority::Medium);
        let callback = imp.on_selected.borrow().as_ref().map(Callback::get);
        if let Some(callback) = callback {
            callback(day);
        }
        self.queue_draw();
    }

    /// The selected day, if any.
    pub fn selected(&self) -> Option<usize> {
        self.imp().selected.get()
    }

    /// Moves the selection one `step` (keys and tests).
    pub fn step(&self, step: layout::Step) {
        let days = self.imp().model.borrow().bars.len();
        if let Some(day) = layout::step(step, self.selected(), days) {
            self.select(day);
        }
    }
}

/// The keys of a chart and the step each makes, without Ctrl, Alt or
/// Shift. The Keyboard Shortcuts dialog lists them (a test checks it).
pub(crate) const KEYS: [(gtk::gdk::Key, layout::Step); 8] = {
    use gtk::gdk::Key;
    use layout::Step;
    [
        (Key::Left, Step::Earlier),
        (Key::KP_Left, Step::Earlier),
        (Key::Right, Step::Later),
        (Key::KP_Right, Step::Later),
        (Key::Home, Step::First),
        (Key::KP_Home, Step::First),
        (Key::End, Step::Last),
        (Key::KP_End, Step::Last),
    ]
};

/// What a key does to the selection.
pub fn key_step(key: gtk::gdk::Key) -> Option<layout::Step> {
    KEYS.iter()
        .find(|(bound, _)| *bound == key)
        .map(|&(_, step)| step)
}

fn source(cr: &gtk::cairo::Context, color: &gtk::gdk::RGBA, opacity: f64) {
    cr.set_source_rgba(
        f64::from(color.red()),
        f64::from(color.green()),
        f64::from(color.blue()),
        f64::from(color.alpha()) * opacity,
    );
}

fn with_opacity(color: &gtk::gdk::RGBA, opacity: f64) -> gtk::gdk::RGBA {
    #[expect(
        clippy::cast_possible_truncation,
        reason = "an opacity in 0..=1 fits an f32"
    )]
    let alpha = (f64::from(color.alpha()) * opacity) as f32;
    color.with_alpha(alpha)
}

/// The path of a rectangle with its top corners rounded by `radius`.
fn rounded_top(cr: &gtk::cairo::Context, r: layout::Rect, radius: f64) {
    use std::f64::consts::{FRAC_PI_2, PI};
    let (left, top, right, bottom) = (r.x, r.y, r.x + r.width, r.y + r.height);
    cr.new_sub_path();
    cr.move_to(left, bottom);
    cr.line_to(left, top + radius);
    cr.arc(left + radius, top + radius, radius, PI, PI + FRAC_PI_2);
    cr.line_to(right - radius, top);
    cr.arc(right - radius, top + radius, radius, -FRAC_PI_2, 0.0);
    cr.line_to(right, bottom);
    cr.close_path();
}

/// The path of a rectangle with all corners rounded by `radius`.
fn rounded(cr: &gtk::cairo::Context, r: layout::Rect, radius: f64) {
    use std::f64::consts::{FRAC_PI_2, PI};
    let radius = radius.min(r.width / 2.0).min(r.height / 2.0).max(0.0);
    let (left, top, right, bottom) = (r.x, r.y, r.x + r.width, r.y + r.height);
    cr.new_sub_path();
    cr.arc(left + radius, top + radius, radius, PI, PI + FRAC_PI_2);
    cr.arc(right - radius, top + radius, radius, -FRAC_PI_2, 0.0);
    cr.arc(right - radius, bottom - radius, radius, 0.0, FRAC_PI_2);
    cr.arc(left + radius, bottom - radius, radius, FRAC_PI_2, PI);
    cr.close_path();
}

/// The legend's sample of a bar part: the accent colour, full for the
/// lower part (Passed) or at the upper part's opacity (Failed), with
/// rounded corners. The colours are read now.
pub fn paint_swatch(cr: &gtk::cairo::Context, width: i32, height: i32, upper: bool) {
    let rect = layout::Rect {
        x: 0.0,
        y: 0.0,
        width: f64::from(width),
        height: f64::from(height),
    };
    let accent = adw::StyleManager::default().accent_color_rgba();
    source(cr, &accent, if upper { UPPER_OPACITY } else { 1.0 });
    rounded(cr, rect, 3.0);
    if let Err(error) = cr.fill() {
        tracing::debug!(%error, "could not draw a legend swatch");
    }
}

/// Redraws a widget when the accent colour or the dark mode changes, for
/// as long as it lives (the handlers are disconnected on drop).
#[derive(Default)]
pub struct StyleWatch(Vec<glib::SignalHandlerId>);

impl std::fmt::Debug for StyleWatch {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("StyleWatch")
    }
}

impl StyleWatch {
    pub fn new(widget: &impl IsA<gtk::Widget>) -> Self {
        let manager = adw::StyleManager::default();
        let weak = widget.upcast_ref::<gtk::Widget>().downgrade();
        let redraw = move |_: &adw::StyleManager| {
            if let Some(widget) = weak.upgrade() {
                widget.queue_draw();
            }
        };
        Self(vec![
            manager.connect_accent_color_rgba_notify(redraw.clone()),
            manager.connect_dark_notify(redraw),
        ])
    }
}

impl Drop for StyleWatch {
    fn drop(&mut self) {
        let manager = adw::StyleManager::default();
        for id in self.0.drain(..) {
            manager.disconnect(id);
        }
    }
}

/// Where the parts of a chart go in a widget of a given size.
struct Frame {
    /// The bars' area; the value labels are at its right, the dates under.
    plot: layout::Rect,
    /// Each grid line's height and its label.
    ticks: Vec<(f64, pango::Layout)>,
    /// Each date's centre and its text.
    dates: Vec<(f64, pango::Layout)>,
}

mod imp {
    use adw::prelude::*;
    use gtk::cairo;
    use gtk::gdk;
    use gtk::glib;
    use gtk::graphene;
    use gtk::pango;
    use gtk::subclass::prelude::*;
    use std::cell::{Cell, RefCell};

    use super::layout::{self, Bar};
    use super::{
        BASE_OPACITY, ChartModel, Frame, GRID_OPACITY, HIGHLIGHT_OPACITY, HIGHLIGHT_RADIUS,
        LABEL_GAP, LABEL_SCALE, OnSelected, PLOT_HEIGHT, StyleWatch, TEXT_OPACITY, UPPER_OPACITY,
        key_step, rounded, rounded_top, source, with_opacity,
    };
    use crate::ui::callback::Callback;

    #[derive(Debug, Default)]
    pub struct BarChart {
        pub model: RefCell<ChartModel>,
        pub selected: Cell<Option<usize>>,
        pub on_selected: RefCell<Option<Callback<OnSelected>>>,
        style_watch: RefCell<StyleWatch>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for BarChart {
        const NAME: &'static str = "RappelBarChart";
        type Type = super::BarChart;
        type ParentType = gtk::DrawingArea;

        fn class_init(klass: &mut Self::Class) {
            klass.set_accessible_role(gtk::AccessibleRole::Img);
        }
    }

    impl ObjectImpl for BarChart {
        fn constructed(&self) {
            self.parent_constructed();
            let chart = self.obj();
            chart.set_content_height(self.height());
            chart.set_hexpand(true);
            chart.set_focusable(true);
            chart.add_css_class("bar-chart");

            chart.set_draw_func(glib::clone!(
                #[weak]
                chart,
                move |_, cr, width, height| {
                    if let Err(error) = chart.imp().paint(cr, width, height) {
                        tracing::debug!(%error, "could not draw a chart");
                    }
                }
            ));

            let keys = gtk::EventControllerKey::new();
            keys.connect_key_pressed(glib::clone!(
                #[weak]
                chart,
                #[upgrade_or]
                glib::Propagation::Proceed,
                move |_, key, _, state| {
                    // Leave Ctrl/Alt/Shift shortcuts to the window.
                    let plain = !state.intersects(
                        gdk::ModifierType::CONTROL_MASK
                            | gdk::ModifierType::ALT_MASK
                            | gdk::ModifierType::SHIFT_MASK,
                    );
                    match key_step(key).filter(|_| plain) {
                        Some(step) => {
                            chart.step(step);
                            glib::Propagation::Stop
                        }
                        None => glib::Propagation::Proceed,
                    }
                }
            ));
            chart.add_controller(keys);

            // A press (mouse, pen or touch) anywhere on the chart selects
            // the nearest day, on release rather than press: a swipe that
            // starts on the chart and scrolls the page has its touch
            // sequence claimed by the scrolled window, which cancels it here
            // ("the gesture must forget everything about the sequence",
            // Gtk-4.0.gir, `GtkGesture::cancel`), so it should not select a
            // day. Not checked on a touch screen: see ADR 0078.
            let click = gtk::GestureClick::new();
            click.connect_released(glib::clone!(
                #[weak]
                chart,
                move |_, _, x, _| {
                    let imp = chart.imp();
                    let plot = imp.frame(chart.width(), chart.height()).plot;
                    let days = imp.model.borrow().bars.len();
                    chart.grab_focus();
                    if let Some(day) = layout::day_near(x - plot.x, plot.width, days) {
                        chart.select(day);
                    }
                }
            ));
            chart.add_controller(click);

            self.style_watch.replace(StyleWatch::new(&*chart));
        }
    }

    impl WidgetImpl for BarChart {
        /// The text size changed (the font, its DPI or the font
        /// configuration: Gtk-4.0.gir, `GtkSystemSetting`): the labels'
        /// lines are taller or shorter, so the chart's height follows. The
        /// parent's handler updates the widget's Pango context first.
        fn system_setting_changed(&self, setting: &gtk::SystemSetting) {
            self.parent_system_setting_changed(setting);
            if matches!(
                setting,
                gtk::SystemSetting::Dpi
                    | gtk::SystemSetting::FontName
                    | gtk::SystemSetting::FontConfig
            ) {
                let chart = self.obj();
                chart.set_content_height(self.height());
                chart.queue_draw();
            }
        }

        /// A new window may have another display and font settings.
        fn root(&self) {
            self.parent_root();
            self.obj().set_content_height(self.height());
        }

        /// The drawing area paints the shapes (`paint`, through the draw
        /// function, which its own snapshot calls whenever the widget is
        /// drawn again: Gtk-4.0.gir, `GtkDrawingArea`); the texts go on
        /// top as Pango layouts made by the widget, so in its font, which
        /// `gtk_snapshot_append_layout` renders at the current origin in
        /// the colour given (Gtk-4.0.gir, gtk/gskpango.c).
        fn snapshot(&self, snapshot: &gtk::Snapshot) {
            self.parent_snapshot(snapshot);
            let chart = self.obj();
            let frame = self.frame(chart.width(), chart.height());
            let color = with_opacity(&chart.color(), TEXT_OPACITY);
            let draw = |x: f64, y: f64, text: &pango::Layout| {
                snapshot.save();
                #[expect(
                    clippy::cast_possible_truncation,
                    reason = "widget coordinates fit an f32"
                )]
                snapshot.translate(&graphene::Point::new(x as f32, y as f32));
                snapshot.append_layout(text, &color);
                snapshot.restore();
            };
            let label_x = frame.plot.x + frame.plot.width + LABEL_GAP;
            for (y, text) in &frame.ticks {
                let (_, height) = text.pixel_size();
                draw(label_x, y - f64::from(height) / 2.0, text);
            }
            let date_y = frame.plot.y + frame.plot.height + LABEL_GAP / 2.0;
            for (centre, text) in &frame.dates {
                let (width, _) = text.pixel_size();
                let x = (centre - f64::from(width) / 2.0).max(0.0);
                draw(x, date_y, text);
            }
        }
    }

    impl DrawingAreaImpl for BarChart {}

    impl BarChart {
        /// A label in the chart's text: libadwaita's caption size, with
        /// figures of equal width (`.numeric`).
        fn label(&self, text: &str) -> pango::Layout {
            let layout = self.obj().create_pango_layout(Some(text));
            let attributes = pango::AttrList::new();
            attributes.insert(pango::AttrFloat::new_scale(LABEL_SCALE));
            attributes.insert(pango::AttrFontFeatures::new("tnum=1"));
            layout.set_attributes(Some(&attributes));
            layout
        }

        /// The height of one line of labels.
        fn line_height(&self) -> f64 {
            f64::from(self.label("0").pixel_size().1)
        }

        /// The whole height the chart needs: half a label above the top
        /// line, the bars, and a line of dates.
        pub(super) fn height(&self) -> i32 {
            let line = self.line_height();
            let total = line / 2.0 + PLOT_HEIGHT + LABEL_GAP / 2.0 + line;
            #[expect(
                clippy::cast_possible_truncation,
                reason = "a few hundred pixels at most"
            )]
            let total = total.ceil().clamp(0.0, 2000.0) as i32;
            total
        }

        pub(super) fn frame(&self, width: i32, height: i32) -> Frame {
            let model = self.model.borrow();
            let line = self.line_height();
            let ticks: Vec<(f64, pango::Layout)> = model
                .ticks
                .iter()
                .map(|t| (t.value, self.label(&t.label)))
                .collect();
            let label_width = ticks
                .iter()
                .map(|(_, l)| f64::from(l.pixel_size().0))
                .fold(0.0, f64::max);
            let (w, h) = (f64::from(width), f64::from(height));
            let plot = layout::Rect {
                x: 0.0,
                y: line / 2.0,
                width: (w - label_width - LABEL_GAP).max(0.0),
                height: (h - line / 2.0 - LABEL_GAP / 2.0 - line).max(0.0),
            };
            let ceiling = model.ceiling();
            let ticks = ticks
                .into_iter()
                .map(|(value, l)| (plot.y + plot.height * (1.0 - value / ceiling), l))
                .collect();
            let days = model.bars.len();
            let dates = model
                .dates
                .iter()
                .enumerate()
                .filter_map(|(i, d)| d.as_ref().map(|d| (i, d)))
                .map(|(i, d)| {
                    (
                        plot.x + layout::slot_centre(i, plot.width, days),
                        self.label(d),
                    )
                })
                .collect();
            Frame { plot, ticks, dates }
        }

        pub(super) fn paint(
            &self,
            cr: &cairo::Context,
            width: i32,
            height: i32,
        ) -> Result<(), cairo::Error> {
            let chart = self.obj();
            let frame = self.frame(width, height);
            let plot = frame.plot;
            let foreground = chart.color();
            let accent = adw::StyleManager::default().accent_color_rgba();
            let model = self.model.borrow();
            let data: Vec<Bar> = model.bars.iter().map(|d| d.bar).collect();
            let rects = layout::bars(&data, plot.width, plot.height, model.ceiling());

            // The grid: a line at each value, the base line stronger.
            cr.set_line_width(1.0);
            for (i, (y, _)) in frame.ticks.iter().enumerate() {
                let y = y.round() - 0.5;
                source(
                    cr,
                    &foreground,
                    if i == 0 { BASE_OPACITY } else { GRID_OPACITY },
                );
                cr.move_to(plot.x, y);
                cr.line_to(plot.x + plot.width, y);
                cr.stroke()?;
            }

            cr.translate(plot.x, plot.y);
            if let Some(slot) = self
                .selected
                .get()
                .and_then(|d| rects.get(d))
                .map(|r| r.slot)
            {
                source(cr, &foreground, HIGHLIGHT_OPACITY);
                let inset = layout::Rect {
                    x: slot.x + 1.0,
                    width: (slot.width - 2.0).max(0.0),
                    ..slot
                };
                rounded(cr, inset, HIGHLIGHT_RADIUS);
                cr.fill()?;
            }

            for r in &rects {
                if let Some(lower) = r.lower {
                    // Square on top when an upper part stands on it.
                    let radius = if r.upper.is_some() {
                        0.0
                    } else {
                        layout::corner_radius(lower.width, lower.height)
                    };
                    source(cr, &accent, 1.0);
                    rounded_top(cr, lower, radius);
                    cr.fill()?;
                }
                if let Some(upper) = r.upper {
                    source(cr, &accent, UPPER_OPACITY);
                    rounded_top(cr, upper, layout::corner_radius(upper.width, upper.height));
                    cr.fill()?;
                }
            }
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::rc::Rc;

    use gtk::cairo::{Context, Format, ImageSurface};

    use super::*;

    fn model(values: &[(f64, f64)], ceiling: f64) -> ChartModel {
        ChartModel {
            bars: values
                .iter()
                .enumerate()
                .map(|(i, &(lower, upper))| ChartBar {
                    bar: Bar { lower, upper },
                    caption: format!("day {i}"),
                })
                .collect(),
            ticks: vec![
                Tick {
                    value: 0.0,
                    label: "0".to_owned(),
                },
                Tick {
                    value: ceiling,
                    label: format!("{ceiling}"),
                },
            ],
            dates: values
                .iter()
                .enumerate()
                .map(|(i, _)| (i % 7 == 0).then(|| format!("{i}")))
                .collect(),
            label: "Reviews".to_owned(),
            summary: "Summary".to_owned(),
        }
    }

    /// Paints `chart` at `width` × its height and returns the pixels'
    /// alpha bytes.
    fn painted(chart: &BarChart, width: i32) -> Vec<u8> {
        let height = chart.imp().height();
        let mut surface = ImageSurface::create(Format::ARgb32, width, height).unwrap();
        {
            let cr = Context::new(&surface).unwrap();
            chart.imp().paint(&cr, width, height).unwrap();
        }
        surface.flush();
        let stride = usize::try_from(surface.stride()).unwrap();
        let data = surface.data().unwrap();
        let row_bytes = usize::try_from(width).unwrap() * 4;
        (0..usize::try_from(height).unwrap())
            .flat_map(|y| (0..row_bytes).step_by(4).map(move |x| (y, x)))
            .map(|(y, x)| data[y * stride + x + 3])
            .collect()
    }

    #[test]
    fn keys_move_the_selection() {
        use gtk::gdk::Key;
        assert_eq!(key_step(Key::Left), Some(layout::Step::Earlier));
        assert_eq!(key_step(Key::Right), Some(layout::Step::Later));
        assert_eq!(key_step(Key::Home), Some(layout::Step::First));
        assert_eq!(key_step(Key::End), Some(layout::Step::Last));
        assert_eq!(key_step(Key::Up), None);
        assert_eq!(key_step(Key::space), None);
    }

    #[test]
    fn stepping_selects_a_day_and_tells_the_page() {
        crate::testing::with_widgets(|| {
            let chart = BarChart::default();
            let heard: Rc<RefCell<Vec<usize>>> = Rc::default();
            let sink = Rc::clone(&heard);
            chart.connect_selected(move |d| sink.borrow_mut().push(d));
            chart.set_model(model(&[(1.0, 0.0), (2.0, 1.0), (3.0, 0.0)], 5.0), Some(2));
            // Setting the model is silent.
            assert_eq!(chart.selected(), Some(2));
            assert!(heard.borrow().is_empty());
            chart.step(layout::Step::Earlier);
            assert_eq!(chart.selected(), Some(1));
            chart.step(layout::Step::Earlier);
            chart.step(layout::Step::Earlier);
            assert_eq!(chart.selected(), Some(0));
            chart.step(layout::Step::Last);
            assert_eq!(chart.selected(), Some(2));
            assert_eq!(*heard.borrow(), vec![1, 0, 0, 2]);
            // The accessible label and description come from the model.
            chart.set_model(model(&[(1.0, 0.0)], 1.0), None);
            assert_eq!(chart.selected(), None);
        });
    }

    #[test]
    fn a_day_that_does_not_exist_cannot_be_selected() {
        crate::testing::with_widgets(|| {
            let chart = BarChart::default();
            chart.set_model(model(&[(1.0, 0.0)], 1.0), Some(5));
            assert_eq!(chart.selected(), None);
            chart.select(5);
            assert_eq!(chart.selected(), None);
            chart.step(layout::Step::Later);
            assert_eq!(chart.selected(), Some(0));
        });
    }

    #[test]
    fn the_chart_draws_at_a_phone_width_and_a_wide_one() {
        crate::testing::with_widgets(|| {
            let chart = BarChart::default();
            let thirty: Vec<(f64, f64)> = (0..30)
                .map(|i| (f64::from(i) * 3.0, f64::from(i % 4)))
                .collect();
            let cases = [
                model(&[], 1.0),
                model(&[(0.0, 0.0)], 1.0),
                model(&[(1.0e9, 1.0e9)], 1.0),
                model(&thirty, 100.0),
            ];
            for m in cases {
                chart.set_model(m, Some(0));
                for width in [296, 1200] {
                    // The grid lines are always drawn.
                    assert!(painted(&chart, width).iter().any(|&a| a > 0));
                }
                // Narrower than its labels: nothing to draw, no panic.
                let _ = painted(&chart, 10);
            }
        });
    }

    #[test]
    fn bars_are_drawn_on_top_of_an_empty_chart() {
        crate::testing::with_widgets(|| {
            let chart = BarChart::default();
            chart.set_model(model(&[], 20.0), None);
            let empty = painted(&chart, 296);
            chart.set_model(model(&[(10.0, 5.0); 30], 20.0), None);
            let full = painted(&chart, 296);
            let filled = |pixels: &[u8]| pixels.iter().filter(|&&a| a > 0).count();
            assert!(filled(&full) > filled(&empty) + 1000);
        });
    }

    #[test]
    fn the_failed_part_is_fainter_than_the_passed_part() {
        crate::testing::with_widgets(|| {
            let chart = BarChart::default();
            // One day: half passed, half failed, filling the axis.
            chart.set_model(model(&[(10.0, 10.0)], 20.0), None);
            let width = 200;
            let pixels = painted(&chart, width);
            let frame = chart.imp().frame(width, chart.imp().height());
            let column = |y: f64| {
                #[expect(
                    clippy::cast_possible_truncation,
                    clippy::cast_sign_loss,
                    reason = "pixel positions inside the surface"
                )]
                let (x, y) = (
                    (frame.plot.width / 2.0) as usize,
                    (frame.plot.y + y) as usize,
                );
                pixels[y * usize::try_from(width).unwrap() + x]
            };
            let lower = column(frame.plot.height * 0.75);
            let upper = column(frame.plot.height * 0.25);
            assert!(lower > 240, "{lower}");
            assert!(upper > 60 && upper < 140, "{upper}");
        });
    }
}
