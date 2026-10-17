//! The geometry and keys of a bar chart, pure and unit-tested: the widget
//! only turns these numbers into Cairo calls and GTK events.

/// One day: a solid part (passes, or the only series) and, stacked above
/// it, a hatched part (fails). Both are zero or more, in the chart's unit.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Bar {
    pub solid: f64,
    pub hatched: f64,
}

impl Bar {
    pub fn total(&self) -> f64 {
        self.solid + self.hatched
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

/// Where one day is drawn: its whole slot (the area a click or a selection
/// outline covers) and the parts of its bar, if it has any.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BarRects {
    pub slot: Rect,
    pub solid: Option<Rect>,
    pub hatched: Option<Rect>,
}

/// Space between the solid and the hatched part, so they stay two shapes
/// without relying on colour.
pub const GAP: f64 = 1.0;
/// A non-zero value is at least this tall, so a single review is visible.
pub const MIN_BAR_HEIGHT: f64 = 1.0;
/// The share of a day's slot its bar fills.
const BAR_SHARE: f64 = 0.8;

/// A count as a float; the counts here are tiny (days).
fn to_f64(n: usize) -> f64 {
    f64::from(u32::try_from(n).unwrap_or(u32::MAX))
}

/// The top of the value axis for a largest value of `max`: the next of
/// 1, 2, 5, 10, 20, 50, … at or above it, and at least 1 (so an empty chart
/// has a scale and the axis never divides by zero).
pub fn nice_ceiling(max: f64) -> f64 {
    if !max.is_finite() || max <= 1.0 {
        return 1.0;
    }
    let magnitude = 10f64.powf(max.log10().floor());
    [1.0, 2.0, 5.0, 10.0]
        .into_iter()
        .map(|step| step * magnitude)
        .find(|&ceiling| ceiling >= max)
        .unwrap_or(10.0 * magnitude)
}

/// The rectangles of `data` in an area of `width` × `height`, bottom
/// aligned, with the value axis running from 0 to `ceiling`. Values beyond
/// the ceiling are cut to the area; nothing is ever drawn outside it.
pub fn bars(data: &[Bar], width: f64, height: f64, ceiling: f64) -> Vec<BarRects> {
    let (width, height) = (width.max(0.0), height.max(0.0));
    let ceiling = if ceiling > 0.0 { ceiling } else { 1.0 };
    let slot_width = width / to_f64(data.len().max(1));
    let bar_width = (slot_width * BAR_SHARE).max(1.0).min(slot_width);
    let scale = |value: f64| {
        if value > 0.0 {
            (value / ceiling * height).max(MIN_BAR_HEIGHT)
        } else {
            0.0
        }
    };
    data.iter()
        .enumerate()
        .map(|(i, bar)| {
            let left = to_f64(i) * slot_width;
            let x = left + (slot_width - bar_width) / 2.0;
            let (mut solid, mut hatched) = (scale(bar.solid), scale(bar.hatched));
            let gap = if solid > 0.0 && hatched > 0.0 {
                GAP
            } else {
                0.0
            };
            if solid + gap + hatched > height {
                let factor = ((height - gap) / (solid + hatched)).max(0.0);
                solid *= factor;
                hatched *= factor;
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
                solid: part(height, solid),
                hatched: part(height - solid - gap, hatched),
            }
        })
        .collect()
}

/// The day under horizontal position `x` of a chart `width` wide with
/// `days` days, if any.
pub fn day_at(x: f64, width: f64, days: usize) -> Option<usize> {
    if days == 0 || width.is_nan() || width <= 0.0 || !(0.0..width).contains(&x) {
        return None;
    }
    // Truncation is the floor of a non-negative number below `days`.
    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "0 <= x / width * days < days"
    )]
    let day = (x / width * to_f64(days)) as usize;
    Some(day.min(days - 1))
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
/// Earlier and Last choose the last day (today, or the farthest forecast
/// day) and the others the first. Stepping past an end stays on it.
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

    fn bar(solid: f64, hatched: f64) -> Bar {
        Bar { solid, hatched }
    }

    #[test]
    fn the_ceiling_is_the_next_round_number() {
        for (max, ceiling) in [
            (-3.0, 1.0),
            (0.0, 1.0),
            (1.0, 1.0),
            (1.5, 2.0),
            (2.0, 2.0),
            (3.0, 5.0),
            (5.0, 5.0),
            (6.0, 10.0),
            (38.0, 50.0),
            (412.0, 500.0),
            (1000.0, 1000.0),
            (1001.0, 2000.0),
        ] {
            assert_eq!(nice_ceiling(max), ceiling, "max {max}");
        }
        assert_eq!(nice_ceiling(f64::NAN), 1.0);
        assert_eq!(nice_ceiling(f64::INFINITY), 1.0);
    }

    #[test]
    fn bars_stand_on_the_bottom_with_the_hatched_part_above() {
        let rects = bars(&[bar(10.0, 5.0)], 100.0, 100.0, 20.0);
        let r = &rects[0];
        let solid = r.solid.unwrap();
        let hatched = r.hatched.unwrap();
        assert_eq!((solid.y + solid.height).round(), 100.0);
        assert_eq!(solid.height.round(), 50.0);
        assert_eq!(hatched.height.round(), 25.0);
        // One gap between them.
        assert!((solid.y - (hatched.y + hatched.height) - GAP).abs() < 1e-9);
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
        assert_eq!((rects[0].solid, rects[0].hatched), (None, None));
        assert_eq!(rects[1].solid.unwrap().height, MIN_BAR_HEIGHT);
        assert!(rects[1].hatched.is_none());
        assert!(rects[2].solid.is_none());
        assert_eq!(rects[2].hatched.unwrap().height, MIN_BAR_HEIGHT);
    }

    #[test]
    fn many_days_in_a_narrow_chart_stay_visible_and_inside() {
        // 30 days at 296 px, as at a 360 px window.
        let data = vec![bar(3.0, 1.0); 30];
        let rects = bars(&data, 296.0, 160.0, 5.0);
        let width = rects[0].solid.unwrap().width;
        assert!(width > 7.0 && width < 9.9, "{width}");
        assert!(rects[29].slot.x + rects[29].slot.width <= 296.0 + 1e-9);
    }

    #[test]
    fn no_data_gives_no_rectangles() {
        assert_eq!(bars(&[], 100.0, 100.0, 1.0).len(), 0);
        assert_eq!(day_at(10.0, 100.0, 0), None);
    }

    #[test]
    fn a_position_finds_its_day() {
        assert_eq!(day_at(0.0, 100.0, 10), Some(0));
        assert_eq!(day_at(9.9, 100.0, 10), Some(0));
        assert_eq!(day_at(10.0, 100.0, 10), Some(1));
        assert_eq!(day_at(99.9, 100.0, 10), Some(9));
        assert_eq!(day_at(100.0, 100.0, 10), None);
        assert_eq!(day_at(-1.0, 100.0, 10), None);
        assert_eq!(day_at(5.0, 0.0, 10), None);
        assert_eq!(day_at(f64::NAN, 100.0, 10), None);
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
            let ceiling = nice_ceiling(max);
            let rects = bars(&data, width, height, ceiling);
            prop_assert_eq!(rects.len(), data.len());
            let eps = 1e-6;
            for r in &rects {
                for rect in [r.solid, r.hatched].into_iter().flatten() {
                    prop_assert!(rect.x >= -eps && rect.x + rect.width <= width + eps);
                    prop_assert!(rect.y >= -eps && rect.y + rect.height <= height + eps);
                    prop_assert!(rect.height > 0.0);
                }
                if let (Some(s), Some(h)) = (r.solid, r.hatched) {
                    prop_assert!(h.y + h.height <= s.y + eps);
                }
            }
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
