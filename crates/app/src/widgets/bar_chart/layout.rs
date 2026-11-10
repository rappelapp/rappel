//! The geometry and keys of a bar chart, pure and unit-tested: the widget
//! only turns these numbers into Cairo calls and GTK events (ADR 0078).

/// One day: a lower part (passes, or the only series) and, stacked above
/// it, an upper part (fails). Both are zero or more, in the chart's unit.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Bar {
    pub lower: f64,
    pub upper: f64,
}

impl Bar {
    pub fn total(&self) -> f64 {
        self.lower + self.upper
    }
}

/// A rectangle in widget coordinates, y growing downwards.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Rect {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

/// Where one day is drawn: its whole slot (the area a click selects and
/// the selection's highlight covers) and the parts of its bar, if any.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BarRects {
    pub slot: Rect,
    pub lower: Option<Rect>,
    pub upper: Option<Rect>,
}

/// Space between the lower and the upper part, so they stay two shapes
/// without relying on colour (the brief's 2 px).
pub const GAP: f64 = 2.0;
/// A non-zero lower part is at least this tall, so a single review shows.
pub const MIN_LOWER: f64 = 1.0;
/// A non-zero upper part is at least this tall, so its rounded top and
/// the gap under it can be seen.
pub const MIN_UPPER: f64 = 3.0;
/// The share of a day's slot its bar fills: about 7 px at a 360 px window
/// (30 slots of about 9.5 px), 13 px at 1000 px.
const BAR_SHARE: f64 = 0.72;
/// The largest radius of a bar's rounded top.
const MAX_RADIUS: f64 = 4.0;

/// A count as a float; the counts here are tiny (days).
fn to_f64(n: usize) -> f64 {
    f64::from(u32::try_from(n).unwrap_or(u32::MAX))
}

/// The value axis: from 0 to `ceiling`, with a grid line every `step`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Ticks {
    pub ceiling: f64,
    pub step: f64,
}

impl Ticks {
    /// The values with a grid line, 0 first and `ceiling` last.
    pub fn values(&self) -> Vec<f64> {
        let lines = (self.ceiling / self.step).round().clamp(1.0, 10.0);
        #[expect(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            reason = "clamped to 1..=10 first"
        )]
        let lines = lines as u32;
        (0..=lines).map(|i| f64::from(i) * self.step).collect()
    }
}

/// The axis for a largest value of `max`: a step of 1, 2 or 5 times a
/// power of ten, at least 1 (the values are counts or whole minutes), so
/// that at most four grid lines stand above the base; the ceiling is the
/// first of them at or above `max`. No data, or a value that is not a
/// number, gives a step and a ceiling of 1, so the axis never divides by
/// zero.
pub fn ticks(max: f64) -> Ticks {
    if !max.is_finite() || max <= 1.0 {
        return Ticks {
            ceiling: 1.0,
            step: 1.0,
        };
    }
    let raw = max / 4.0;
    let magnitude = 10f64.powf(raw.log10().floor());
    let step = [1.0, 2.0, 5.0, 10.0]
        .into_iter()
        .map(|s| s * magnitude)
        .find(|&s| s >= raw)
        .unwrap_or(10.0 * magnitude)
        .max(1.0);
    Ticks {
        ceiling: ((max / step).ceil() * step).max(step),
        step,
    }
}

/// The radius of the rounded top of a part `width` wide and `height`
/// tall: at most 4 px, half the width, and the height.
pub fn corner_radius(width: f64, height: f64) -> f64 {
    MAX_RADIUS.min(width / 2.0).min(height).max(0.0)
}

/// The rectangles of `data` in an area of `width` × `height`, bottom
/// aligned, with the value axis running from 0 to `ceiling`. Values beyond
/// the ceiling are cut to the area; nothing is ever drawn outside it.
pub fn bars(data: &[Bar], width: f64, height: f64, ceiling: f64) -> Vec<BarRects> {
    let (width, height) = (width.max(0.0), height.max(0.0));
    let ceiling = if ceiling > 0.0 { ceiling } else { 1.0 };
    let slot_width = width / to_f64(data.len().max(1));
    let bar_width = (slot_width * BAR_SHARE).max(1.0).min(slot_width);
    let scale = |value: f64, least: f64| {
        if value > 0.0 {
            (value / ceiling * height).max(least)
        } else {
            0.0
        }
    };
    data.iter()
        .enumerate()
        .map(|(i, bar)| {
            let left = to_f64(i) * slot_width;
            let x = left + (slot_width - bar_width) / 2.0;
            let (mut lower, mut upper) = (scale(bar.lower, MIN_LOWER), scale(bar.upper, MIN_UPPER));
            let gap = if lower > 0.0 && upper > 0.0 { GAP } else { 0.0 };
            if lower + gap + upper > height {
                let factor = ((height - gap) / (lower + upper)).max(0.0);
                lower *= factor;
                upper *= factor;
            }
            let part = |bottom: f64, part_height: f64| {
                (part_height > 0.0).then_some(Rect {
                    x,
                    y: (bottom - part_height).max(0.0),
                    width: bar_width,
                    height: part_height,
                })
            };
            BarRects {
                slot: Rect {
                    x: left,
                    y: 0.0,
                    width: slot_width,
                    height,
                },
                lower: part(height, lower),
                upper: part(height - lower - gap, upper),
            }
        })
        .collect()
}

/// The day nearest to horizontal position `x` of a chart whose bars take
/// `width` with `days` days: a press anywhere on the chart, its value
/// labels and dates included, selects a day, so a finger need not hit a
/// thin bar (the brief's wider hit area). `None` without days or room.
pub fn day_near(x: f64, width: f64, days: usize) -> Option<usize> {
    if days == 0 || width.is_nan() || width <= 0.0 || x.is_nan() {
        return None;
    }
    let x = x.clamp(0.0, width);
    // Truncation is the floor of a non-negative number at most `days`.
    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "0 <= x / width * days <= days"
    )]
    let day = (x / width * to_f64(days)) as usize;
    Some(day.min(days - 1))
}

/// The horizontal centre of day `i`'s slot, for its date.
pub fn slot_centre(i: usize, width: f64, days: usize) -> f64 {
    let slot = width.max(0.0) / to_f64(days.max(1));
    (to_f64(i) + 0.5) * slot
}

/// A way to move the selected day.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Step {
    Earlier,
    Later,
    First,
    Last,
}

/// The day selected after `step`, for `days` days. With nothing selected,
/// Earlier and Last choose the last day and the others the first. Stepping
/// past an end stays on it.
pub fn step(step: Step, selected: Option<usize>, days: usize) -> Option<usize> {
    let last = days.checked_sub(1)?;
    Some(match (step, selected) {
        (Step::First, _) | (Step::Later, None) => 0,
        (Step::Last, _) | (Step::Earlier, None) => last,
        (Step::Later, Some(i)) => (i + 1).min(last),
        (Step::Earlier, Some(i)) => i.saturating_sub(1).min(last),
    })
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;

    use super::*;

    fn bar(lower: f64, upper: f64) -> Bar {
        Bar { lower, upper }
    }

    #[test]
    fn the_axis_has_round_steps_and_at_most_four_lines_above_the_base() {
        for (max, ceiling, step) in [
            (-3.0, 1.0, 1.0),
            (0.0, 1.0, 1.0),
            (1.0, 1.0, 1.0),
            (1.5, 2.0, 1.0),
            (3.0, 3.0, 1.0),
            (7.0, 8.0, 2.0),
            (25.0, 30.0, 10.0),
            (38.0, 40.0, 10.0),
            (54.0, 60.0, 20.0),
            (412.0, 600.0, 200.0),
            (1000.0, 1000.0, 500.0),
        ] {
            assert_eq!(ticks(max), Ticks { ceiling, step }, "max {max}");
        }
        assert_eq!(ticks(f64::NAN).ceiling, 1.0);
        assert_eq!(ticks(f64::INFINITY).ceiling, 1.0);
        assert_eq!(ticks(38.0).values(), vec![0.0, 10.0, 20.0, 30.0, 40.0]);
        assert_eq!(ticks(0.0).values(), vec![0.0, 1.0]);
    }

    #[test]
    fn a_rounded_top_never_exceeds_the_part() {
        assert_eq!(corner_radius(13.0, 50.0), 4.0);
        assert_eq!(corner_radius(7.0, 50.0), 3.5);
        assert_eq!(corner_radius(13.0, 1.0), 1.0);
        assert_eq!(corner_radius(-1.0, 5.0), 0.0);
    }

    #[test]
    fn bars_stand_on_the_bottom_with_the_upper_part_above_a_gap() {
        let rects = bars(&[bar(10.0, 5.0)], 100.0, 100.0, 20.0);
        let r = &rects[0];
        let lower = r.lower.unwrap();
        let upper = r.upper.unwrap();
        assert_eq!((lower.y + lower.height).round(), 100.0);
        assert_eq!(lower.height.round(), 50.0);
        assert_eq!(upper.height.round(), 25.0);
        assert!((lower.y - (upper.y + upper.height) - GAP).abs() < 1e-9);
        assert_eq!(r.slot.width, 100.0);
    }

    #[test]
    fn empty_days_have_no_bar_and_a_tiny_value_is_still_visible() {
        let rects = bars(
            &[bar(0.0, 0.0), bar(1.0, 0.0), bar(0.0, 1.0)],
            90.0,
            100.0,
            1000.0,
        );
        assert_eq!((rects[0].lower, rects[0].upper), (None, None));
        assert_eq!(rects[1].lower.unwrap().height, MIN_LOWER);
        assert!(rects[1].upper.is_none());
        assert!(rects[2].lower.is_none());
        assert_eq!(rects[2].upper.unwrap().height, MIN_UPPER);
    }

    #[test]
    fn thirty_days_at_a_phone_width_are_about_seven_pixels_wide() {
        // The bars' room at a 360 px window: 30 slots in about 282 px.
        let data = vec![bar(3.0, 1.0); 30];
        let rects = bars(&data, 282.0, 150.0, 5.0);
        let width = rects[0].lower.unwrap().width;
        assert!(width > 6.0 && width < 8.0, "{width}");
        assert!(rects[29].slot.x + rects[29].slot.width <= 282.0 + 1e-9);
    }

    #[test]
    fn no_data_gives_no_rectangles() {
        assert_eq!(bars(&[], 100.0, 100.0, 1.0).len(), 0);
        assert_eq!(day_near(10.0, 100.0, 0), None);
    }

    #[test]
    fn a_press_anywhere_finds_the_nearest_day() {
        assert_eq!(day_near(0.0, 100.0, 10), Some(0));
        assert_eq!(day_near(9.9, 100.0, 10), Some(0));
        assert_eq!(day_near(10.0, 100.0, 10), Some(1));
        assert_eq!(day_near(99.9, 100.0, 10), Some(9));
        // Beyond the bars (on the value labels) and before them.
        assert_eq!(day_near(130.0, 100.0, 10), Some(9));
        assert_eq!(day_near(-4.0, 100.0, 10), Some(0));
        assert_eq!(day_near(5.0, 0.0, 10), None);
        assert_eq!(day_near(f64::NAN, 100.0, 10), None);
        assert_eq!(day_near(5.0, f64::NAN, 10), None);
    }

    #[test]
    fn a_date_is_centred_under_its_day() {
        assert_eq!(slot_centre(0, 300.0, 30), 5.0);
        assert_eq!(slot_centre(29, 300.0, 30), 295.0);
        assert_eq!(slot_centre(0, 300.0, 0), 150.0);
    }

    #[test]
    fn steps_stay_inside_the_days() {
        assert_eq!(step(Step::Later, Some(3), 14), Some(4));
        assert_eq!(step(Step::Later, Some(13), 14), Some(13));
        assert_eq!(step(Step::Earlier, Some(0), 14), Some(0));
        assert_eq!(step(Step::Earlier, Some(5), 14), Some(4));
        assert_eq!(step(Step::First, Some(5), 14), Some(0));
        assert_eq!(step(Step::Last, Some(5), 14), Some(13));
        // Nothing selected yet.
        assert_eq!(step(Step::Earlier, None, 30), Some(29));
        assert_eq!(step(Step::Last, None, 30), Some(29));
        assert_eq!(step(Step::Later, None, 30), Some(0));
        assert_eq!(step(Step::First, None, 30), Some(0));
        // No days.
        assert_eq!(step(Step::Later, None, 0), None);
        // A selection left over from more days is pulled back in.
        assert_eq!(step(Step::Later, Some(40), 30), Some(29));
    }

    proptest! {
        #[test]
        fn every_rectangle_is_inside_the_area_and_parts_do_not_overlap(
            values in prop::collection::vec((0.0f64..2000.0, 0.0f64..2000.0), 0..60),
            width in 0.0f64..2000.0,
            height in 0.0f64..400.0,
        ) {
            let data: Vec<Bar> = values.iter().map(|&(s, h)| bar(s, h)).collect();
            let max = data.iter().map(Bar::total).fold(0.0, f64::max);
            let rects = bars(&data, width, height, ticks(max).ceiling);
            prop_assert_eq!(rects.len(), data.len());
            let eps = 1e-6;
            for r in &rects {
                for rect in [r.lower, r.upper].into_iter().flatten() {
                    prop_assert!(rect.x >= -eps && rect.x + rect.width <= width + eps);
                    prop_assert!(rect.y >= -eps && rect.y + rect.height <= height + eps);
                    prop_assert!(rect.height > 0.0);
                }
                if let (Some(s), Some(h)) = (r.lower, r.upper) {
                    prop_assert!(h.y + h.height <= s.y + eps);
                }
            }
        }

        #[test]
        fn the_axis_always_holds_the_largest_value(max in 0.0f64..1e9) {
            let t = ticks(max);
            prop_assert!(t.ceiling >= max && t.step >= 1.0);
            prop_assert!(t.values().len() <= 6);
            prop_assert!(t.values().len() >= 2);
        }

        #[test]
        fn stepping_always_lands_on_a_day(
            which in 0usize..4,
            selected in prop::option::of(0usize..100),
            days in 1usize..60,
        ) {
            let s = [Step::Earlier, Step::Later, Step::First, Step::Last][which];
            let day = step(s, selected, days).unwrap();
            prop_assert!(day < days);
        }
    }
}
