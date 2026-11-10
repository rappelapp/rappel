//! What the statistics page says and draws, as pure functions of the
//! numbers (unit-tested; the page only puts the results in widgets): the
//! headline figures of each view, the chart with its axis, dates and
//! sentences, the Memory rows, and the paging (ADR 0033, 0078). Day names
//! come in through [`DayNames`], so the tests need no time zone.

use crate::i18n::{fill, gettext, ngettext};
use crate::state::{STATS_DAYS, STATS_MAX_PERIOD, StatsData};
use crate::ui::text::{counted, group_digits, hours, minutes, percent, studied_text};
use crate::widgets::bar_chart::layout::{Bar, ticks};
use crate::widgets::bar_chart::{ChartBar, ChartModel, Tick};

/// How a day, given by when it begins, is named.
pub struct DayNames<'a> {
    /// "17 Sep", under a chart.
    pub short: &'a dyn Fn(i64) -> String,
    /// "Tue 17 Sep", for a selected day.
    pub with_weekday: &'a dyn Fn(i64) -> String,
    /// "17 September", in a summary.
    pub long: &'a dyn Fn(i64) -> String,
    /// "17 Sep 2025", for a period of another year.
    pub short_with_year: &'a dyn Fn(i64) -> String,
    /// "17", under a chart within a month already named.
    pub number: &'a dyn Fn(i64) -> String,
    /// `year * 12 + month` (month 1 to 12): the same number for two days
    /// of the same month, and the year in it.
    pub month: &'a dyn Fn(i64) -> i64,
}

/// The three views of the card, as the toggle group names them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum View {
    Reviews,
    Time,
    Forecast,
}

impl View {
    /// The view of a toggle's name; Reviews for anything else.
    pub fn from_name(name: Option<&str>) -> Self {
        match name {
            Some("time") => Self::Time,
            Some("forecast") => Self::Forecast,
            _ => Self::Reviews,
        }
    }

    /// Whether the view shows past days, which page back and forth; the
    /// forecast is always the next 30 days.
    pub fn pages(self) -> bool {
        self != Self::Forecast
    }

    /// The day selected when the view's days are first shown: today, the
    /// last of a period (also after paging), or tomorrow, the first day of
    /// the forecast.
    pub fn first_selected(self) -> usize {
        if self.pages() { STATS_DAYS - 1 } else { 0 }
    }
}

const NO_DATA: &str = "—";

/// The label of the empty state's button, which goes back to the page the
/// statistics were opened from: a deck's page, or the deck list.
pub fn back_label(one_deck: bool) -> String {
    if one_deck {
        gettext("_Back to Deck")
    } else {
        gettext("_Back to Decks")
    }
}

/// Whether there is nothing at all to show: no review ever, no scheduled
/// card and nothing due. The page is then a status page.
pub fn nothing_to_show(data: &StatsData) -> bool {
    let stats = &data.stats;
    data.first_review_at.is_none()
        && stats.scheduled_cards == 0
        && stats.days.iter().all(|d| d.reviews == 0)
        && stats.forecast.iter().all(|&n| n == 0)
}

/// Whether Previous may page back from `period` (0 is the current one) of
/// `data`: only while the first review is before the period's first day,
/// so paging stops at the period that holds it and never runs on through
/// empty months.
pub fn can_page_back(data: &StatsData, period: u32) -> bool {
    let start = data.past_starts.first().copied();
    period < STATS_MAX_PERIOD
        && matches!((data.first_review_at, start), (Some(first), Some(start)) if first < start)
}

/// Whether Next may page forward: not from the current period.
pub fn can_page_forward(period: u32) -> bool {
    period > 0
}

/// The period after a press of Previous (`back`) or Next, kept within
/// 0 and the furthest.
pub fn paged(period: u32, back: bool) -> u32 {
    if back {
        period.saturating_add(1).min(STATS_MAX_PERIOD)
    } else {
        period.saturating_sub(1)
    }
}

/// A Memory row: its title, the value at its end, and what a screen
/// reader says for the value ("no reviews" for "—").
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Row {
    pub title: String,
    pub value: String,
    pub said: String,
}

/// The Memory rows: retention now, and the pass rate and time studied of
/// the current period (`current` is period 0).
pub fn memory(current: &StatsData) -> [Row; 3] {
    let stats = &current.stats;
    let row = |value: Option<String>, title: String, none: String| Row {
        value: value.clone().unwrap_or_else(|| NO_DATA.to_owned()),
        said: value.unwrap_or(none),
        title,
    };
    let reviews = total_u32(stats.days.iter().map(|d| d.reviews));
    let studied_ms = total_ms(stats.days.iter().map(|d| d.duration_ms));
    [
        row(
            stats.expected_retention.map(percent),
            gettext("Expected retention"),
            gettext("no reviewed cards"),
        ),
        row(
            stats.pass_rate.map(percent),
            gettext("Pass rate, last 30 days"),
            gettext("no reviews"),
        ),
        row(
            (reviews > 0).then(|| studied_text(studied_ms)),
            gettext("Time studied, last 30 days"),
            gettext("no reviews"),
        ),
    ]
}

/// One headline figure: "Today" over "54", with "3 failed" under it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Figure {
    pub key: String,
    pub value: String,
    /// What a screen reader says for the value: "54 reviews".
    pub said: String,
    pub detail: Option<String>,
}

/// The two headline figures of `view`: the selected day, then the whole
/// period (or the next 30 days).
pub fn figures(
    view: View,
    data: &StatsData,
    period: u32,
    selected: usize,
    names: &DayNames<'_>,
) -> [Figure; 2] {
    let stats = &data.stats;
    let day = day_key(view, data, period, selected, names);
    let whole = period_name(view, data, period, names);
    match view {
        View::Reviews => {
            let d = stats.days.get(selected).copied().unwrap_or_default();
            let total = total_u32(stats.days.iter().map(|d| d.reviews));
            [
                Figure {
                    key: day,
                    value: number(d.reviews),
                    said: reviews(d.reviews),
                    detail: Some(failed(d.reviews.saturating_sub(d.passes))),
                },
                Figure {
                    key: whole,
                    value: number(total),
                    said: reviews(total),
                    detail: None,
                },
            ]
        }
        View::Time => {
            let ms = stats.days.get(selected).map_or(0, |d| d.duration_ms);
            let total = studied_text(total_ms(stats.days.iter().map(|d| d.duration_ms)));
            [
                Figure {
                    key: day,
                    value: studied_text(ms),
                    said: studied_text(ms),
                    detail: None,
                },
                Figure {
                    key: whole,
                    value: total.clone(),
                    said: total,
                    detail: None,
                },
            ]
        }
        View::Forecast => {
            let n = stats.forecast.get(selected).copied().unwrap_or(0);
            let total = total_u32(stats.forecast.iter().copied());
            [
                Figure {
                    key: day,
                    value: cards(n),
                    said: cards_due(n),
                    detail: None,
                },
                Figure {
                    key: whole,
                    value: cards(total),
                    said: cards_due(total),
                    detail: None,
                },
            ]
        }
    }
}

/// The accessible label of the chart of `view`.
pub fn chart_title(view: View) -> String {
    match view {
        View::Reviews => gettext("Reviews per Day"),
        View::Time => gettext("Time Studied per Day"),
        View::Forecast => gettext("Due Forecast"),
    }
}

/// The chart of `view` for the period `period` of `data`.
pub fn chart(view: View, data: &StatsData, period: u32, names: &DayNames<'_>) -> ChartModel {
    let starts = if view.pages() {
        &data.past_starts
    } else {
        &data.ahead_starts
    };
    let (bars, summary, label): (Vec<ChartBar>, String, fn(f64) -> String) = match view {
        View::Reviews => (
            reviews_bars(data, period, names),
            reviews_summary(data, period, names),
            count_label,
        ),
        View::Time => (
            time_bars(data, period, names),
            time_summary(data, period, names),
            minutes_label,
        ),
        View::Forecast => (
            forecast_bars(data, period, names),
            forecast_summary(data, period, names),
            count_label,
        ),
    };
    let mut bars = bars;
    let mut label = label;
    let max = bars.iter().map(|b| b.bar.total()).fold(0.0, f64::max);
    if view == View::Time && max > HOURS_ABOVE_MINUTES {
        // Whole hours on the scale: "2 h", "4 h", not "100 min".
        for bar in &mut bars {
            bar.bar.lower /= 60.0;
        }
        label = hours_label;
    }
    let axis = ticks(bars.iter().map(|b| b.bar.total()).fold(0.0, f64::max));
    ChartModel {
        ticks: axis
            .values()
            .into_iter()
            .map(|value| Tick {
                value,
                label: label(value),
            })
            .collect(),
        dates: week_dates(starts, names),
        label: chart_title(view),
        summary,
        bars,
    }
}

/// The date under every seventh day, from the first: "7 Oct", then "14",
/// "21", "28", and "4 Nov" where the month changes.
pub fn week_dates(starts: &[i64], names: &DayNames<'_>) -> Vec<Option<String>> {
    let mut month = None;
    starts
        .iter()
        .enumerate()
        .map(|(i, &start)| {
            (i % 7 == 0).then(|| {
                let this = (names.month)(start);
                let text = if month == Some(this) {
                    (names.number)(start)
                } else {
                    (names.short)(start)
                };
                month = Some(this);
                text
            })
        })
        .collect()
}

/// The name of the selected day: "Today" (the last day of the current
/// period), "Tomorrow" (the forecast's first), else "Tue 14 Oct".
fn day_key(view: View, data: &StatsData, period: u32, i: usize, names: &DayNames<'_>) -> String {
    if view.pages() {
        if period == 0 && i + 1 == data.past_starts.len() {
            return gettext("Today");
        }
        data.past_starts
            .get(i)
            .map_or_else(String::new, |&s| (names.with_weekday)(s))
    } else {
        if i == 0 {
            return gettext("Tomorrow");
        }
        data.ahead_starts
            .get(i)
            .map_or_else(String::new, |&s| (names.with_weekday)(s))
    }
}

/// The name of the whole period: "Last 30 Days", "8 Sep – 7 Oct" for an
/// earlier one, or "Next 30 Days" for the forecast.
pub fn period_name(view: View, data: &StatsData, period: u32, names: &DayNames<'_>) -> String {
    if !view.pages() {
        return gettext("Next 30 Days");
    }
    if period == 0 {
        return gettext("Last 30 Days");
    }
    match (data.past_starts.first(), data.past_starts.last()) {
        (Some(&first), Some(&last)) => {
            // Today is the instant before the forecast's first day.
            let today = data.ahead_starts.first().map(|&t| t.saturating_sub(1));
            let other_year = today.is_some_and(|t| {
                year(names, first) != year(names, t) || year(names, last) != year(names, t)
            });
            let name = if other_year {
                names.short_with_year
            } else {
                names.short
            };
            fill(
                // Translators: a period of the statistics; {first} and
                // {last} are its first and last days, as "8 Sep" (or "8
                // Sep 2025" in another year).
                &gettext("{first} – {last}"),
                &[("first", &name(first)), ("last", &name(last))],
            )
        }
        _ => String::new(),
    }
}

/// The year of day `t`, from [`DayNames::month`].
fn year(names: &DayNames<'_>, t: i64) -> i64 {
    (names.month)(t).saturating_sub(1).div_euclid(12)
}

/// Above this many minutes the Time scale is in hours.
const HOURS_ABOVE_MINUTES: f64 = 120.0;

/// The sum of `counts`, stopping at `u32::MAX`: a day's counts come from
/// the database and may each be as large as that (ADR 0050, row 23), and a
/// plain sum would overflow.
fn total_u32(counts: impl IntoIterator<Item = u32>) -> u32 {
    counts.into_iter().fold(0, u32::saturating_add)
}

/// The sum of `durations` in milliseconds, stopping at the ends of `i64`,
/// as [`total_u32`] does.
fn total_ms(durations: impl IntoIterator<Item = i64>) -> i64 {
    durations.into_iter().fold(0, i64::saturating_add)
}

/// "3 reviews", "1 review".
fn reviews(n: u32) -> String {
    let n = n as usize;
    // Translators: {count} is a number of reviews (answers given to cards).
    counted(&ngettext("{count} review", "{count} reviews", n), n)
}

/// "3 failed", under the selected day's reviews.
fn failed(n: u32) -> String {
    let n = n as usize;
    // Translators: {count} is a number of failed reviews on a day ("3
    // failed"), under its number of reviews.
    counted(&ngettext("{count} failed", "{count} failed", n), n)
}

/// "12 cards", a forecast's headline figure.
fn cards(n: u32) -> String {
    let n = n as usize;
    // Translators: {count} is a number of cards falling due ("12 cards").
    counted(&ngettext("{count} card", "{count} cards", n), n)
}

/// "12 cards due".
fn cards_due(n: u32) -> String {
    let n = n as usize;
    // Translators: {count} is a number of cards due on a day.
    counted(&ngettext("{count} card due", "{count} cards due", n), n)
}

/// A count from the statistics, grouped.
fn number(n: u32) -> String {
    group_digits(n as usize)
}

/// A chart value as a whole number for display (values are counts, or
/// minutes of one day, so far below the limit).
fn whole(value: f64) -> usize {
    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "rounded and clamped to 0..=1e9 first"
    )]
    let n = value.round().clamp(0.0, 1e9) as usize;
    n
}

/// A grid line's value: "40".
fn count_label(value: f64) -> String {
    group_digits(whole(value))
}

/// A grid line's value in minutes: "20 min".
fn minutes_label(value: f64) -> String {
    minutes(u64::try_from(whole(value)).unwrap_or(u64::MAX))
}

/// A grid line's value in hours: "2 h".
fn hours_label(value: f64) -> String {
    hours(u64::try_from(whole(value)).unwrap_or(u64::MAX))
}

/// The day with the largest value, the earliest of equal ones; `None` if
/// every value is zero.
fn busiest(values: impl IntoIterator<Item = f64>) -> Option<(usize, f64)> {
    values
        .into_iter()
        .enumerate()
        .filter(|&(_, v)| v > 0.0)
        .fold(None, |best: Option<(usize, f64)>, (i, v)| match best {
            Some((_, b)) if b >= v => best,
            _ => Some((i, v)),
        })
}

/// Milliseconds as minutes (one day is far below `u32::MAX` ms; more is
/// clamped).
fn minutes_of(ms: i64) -> f64 {
    let ms = u32::try_from(ms.clamp(0, i64::from(u32::MAX))).unwrap_or(0);
    f64::from(ms) / 60_000.0
}

/// The long name of day `i`, empty if there is no such day.
fn day_name(starts: &[i64], i: usize, names: &DayNames<'_>) -> String {
    starts.get(i).map_or_else(String::new, |&s| (names.long)(s))
}

fn reviews_bars(data: &StatsData, period: u32, names: &DayNames<'_>) -> Vec<ChartBar> {
    data.stats
        .days
        .iter()
        .enumerate()
        .map(|(i, day)| {
            let failed = day.reviews.saturating_sub(day.passes);
            let when = day_key(View::Reviews, data, period, i, names);
            let caption = if day.reviews == 0 {
                // Translators: a chart's day; {day} is "Tue 14 Oct".
                fill(&gettext("{day}: no reviews"), &[("day", &when)])
            } else {
                fill(
                    // Translators: a chart's day; {day} is "Tue 14 Oct",
                    // {reviews} "12 reviews", {passed} and {failed} numbers.
                    &gettext("{day}: {reviews}, {passed} passed, {failed} failed"),
                    &[
                        ("day", &when),
                        ("reviews", &reviews(day.reviews)),
                        ("passed", &number(day.passes)),
                        ("failed", &number(failed)),
                    ],
                )
            };
            ChartBar {
                bar: Bar {
                    lower: f64::from(day.passes.min(day.reviews)),
                    upper: f64::from(failed),
                },
                caption,
            }
        })
        .collect()
}

fn reviews_summary(data: &StatsData, period: u32, names: &DayNames<'_>) -> String {
    let days = &data.stats.days;
    let name = period_name(View::Reviews, data, period, names);
    let total = total_u32(days.iter().map(|d| d.reviews));
    let passed = total_u32(days.iter().map(|d| d.passes.min(d.reviews)));
    match busiest(days.iter().map(|d| f64::from(d.reviews))) {
        // Translators: a chart's summary for screen readers; {period} is
        // "Last 30 Days" or "8 Sep – 7 Oct".
        None => fill(&gettext("{period}: no reviews."), &[("period", &name)]),
        Some((i, most)) => fill(
            // Translators: a chart's summary for screen readers: {period}
            // is "Last 30 Days" or "8 Sep – 7 Oct", {reviews} "120
            // reviews", {passed} and {failed} numbers, {day} "14 October",
            // {most} a number of reviews.
            &gettext(
                "{period}: {reviews}, {passed} passed, {failed} failed, busiest day {day} with {most}.",
            ),
            &[
                ("period", &name),
                ("reviews", &reviews(total)),
                ("passed", &number(passed)),
                ("failed", &number(total.saturating_sub(passed))),
                ("day", &day_name(&data.past_starts, i, names)),
                ("most", &group_digits(whole(most))),
            ],
        ),
    }
}

fn time_bars(data: &StatsData, period: u32, names: &DayNames<'_>) -> Vec<ChartBar> {
    data.stats
        .days
        .iter()
        .enumerate()
        .map(|(i, day)| {
            let when = day_key(View::Time, data, period, i, names);
            let caption = if day.duration_ms <= 0 {
                // Translators: a chart's day; {day} is "Tue 14 Oct".
                fill(&gettext("{day}: no time studied"), &[("day", &when)])
            } else {
                fill(
                    // Translators: a chart's day; {day} is "Tue 14 Oct",
                    // {time} "12 min".
                    &gettext("{day}: {time} studied"),
                    &[("day", &when), ("time", &studied_text(day.duration_ms))],
                )
            };
            ChartBar {
                bar: Bar {
                    lower: minutes_of(day.duration_ms),
                    upper: 0.0,
                },
                caption,
            }
        })
        .collect()
}

fn time_summary(data: &StatsData, period: u32, names: &DayNames<'_>) -> String {
    let days = &data.stats.days;
    let name = period_name(View::Time, data, period, names);
    let total = total_ms(days.iter().map(|d| d.duration_ms));
    match busiest(days.iter().map(|d| minutes_of(d.duration_ms))) {
        // Translators: a chart's summary for screen readers; {period} is
        // "Last 30 Days" or "8 Sep – 7 Oct".
        None => fill(&gettext("{period}: no time studied."), &[("period", &name)]),
        Some((i, _)) => fill(
            // Translators: a chart's summary for screen readers: {period}
            // is "Last 30 Days" or "8 Sep – 7 Oct", {time} "3 h 12 min",
            // {day} "14 October", {most} "25 min".
            &gettext("{period}: {time} studied, longest day {day} with {most}."),
            &[
                ("period", &name),
                ("time", &studied_text(total)),
                ("day", &day_name(&data.past_starts, i, names)),
                ("most", &studied_text(days[i].duration_ms)),
            ],
        ),
    }
}

fn forecast_bars(data: &StatsData, period: u32, names: &DayNames<'_>) -> Vec<ChartBar> {
    data.stats
        .forecast
        .iter()
        .enumerate()
        .map(|(i, &n)| {
            let when = day_key(View::Forecast, data, period, i, names);
            let caption = if n == 0 {
                // Translators: a chart's day; {day} is "Tue 14 Oct".
                fill(&gettext("{day}: nothing due"), &[("day", &when)])
            } else {
                // Translators: a chart's day; {day} is "Tue 14 Oct", {cards}
                // "12 cards due".
                fill(
                    &gettext("{day}: {cards}"),
                    &[("day", &when), ("cards", &cards_due(n))],
                )
            };
            ChartBar {
                bar: Bar {
                    lower: f64::from(n),
                    upper: 0.0,
                },
                caption,
            }
        })
        .collect()
}

fn forecast_summary(data: &StatsData, period: u32, names: &DayNames<'_>) -> String {
    let forecast = &data.stats.forecast;
    let name = period_name(View::Forecast, data, period, names);
    let total = total_u32(forecast.iter().copied());
    match busiest(forecast.iter().map(|&n| f64::from(n))) {
        // Translators: a chart's summary for screen readers; {period} is
        // "Next 30 Days".
        None => fill(&gettext("{period}: nothing due."), &[("period", &name)]),
        Some((i, most)) => {
            let day = if i == 0 {
                gettext("tomorrow")
            } else {
                day_name(&data.ahead_starts, i, names)
            };
            fill(
                // Translators: a chart's summary for screen readers:
                // {period} is "Next 30 Days", {cards} "40 cards due",
                // {day} "tomorrow" or "14 October", {most} a number of
                // cards.
                &gettext("{period}: {cards}, busiest day {day} with {most}."),
                &[
                    ("period", &name),
                    ("cards", &cards_due(total)),
                    ("day", &day),
                    ("most", &group_digits(whole(most))),
                ],
            )
        }
    }
}

#[cfg(test)]
mod tests {
    use rappel_core::stats::{DayActivity, Stats};

    use super::*;

    /// Days 100.. are in January 2026 until 113, then February; days
    /// before 100 are in December 2025.
    fn names() -> DayNames<'static> {
        DayNames {
            short: &|t| format!("d{t}"),
            with_weekday: &|t| format!("Day {t}"),
            long: &|t| format!("day {t}"),
            short_with_year: &|t| format!("d{t}y"),
            number: &|t| format!("{}", t % 100),
            month: &|t| {
                if t < 100 {
                    2025 * 12 + 12
                } else {
                    2026 * 12 + 1 + i64::from(t >= 114)
                }
            },
        }
    }

    fn day(reviews: u32, passes: u32, duration_ms: i64) -> DayActivity {
        DayActivity {
            reviews,
            passes,
            duration_ms,
        }
    }

    /// 30 period days, starting at 100, and 30 days ahead, starting at 1000.
    fn data(days: Vec<DayActivity>, forecast: Vec<u32>) -> StatsData {
        let n = i64::try_from(days.len()).unwrap();
        let a = i64::try_from(forecast.len()).unwrap();
        let scheduled = total_u32(forecast.iter().copied());
        StatsData {
            stats: Stats {
                days,
                pass_rate: None,
                forecast,
                overdue: 0,
                scheduled_cards: scheduled,
                expected_retention: None,
            },
            past_starts: (0..n).map(|i| 100 + i).collect(),
            ahead_starts: (0..a).map(|i| 1000 + i).collect(),
            first_review_at: None,
        }
    }

    fn quiet() -> StatsData {
        data(vec![day(0, 0, 0); 30], vec![0; 30])
    }

    #[test]
    fn the_empty_state_goes_back_where_it_came_from() {
        assert_eq!(back_label(true), "_Back to Deck");
        assert_eq!(back_label(false), "_Back to Decks");
    }

    #[test]
    fn views_are_named_as_the_toggles_are() {
        assert_eq!(View::from_name(Some("reviews")), View::Reviews);
        assert_eq!(View::from_name(Some("time")), View::Time);
        assert_eq!(View::from_name(Some("forecast")), View::Forecast);
        assert_eq!(View::from_name(None), View::Reviews);
        assert_eq!(View::Reviews.first_selected(), 29);
        assert_eq!(View::Time.first_selected(), 29);
        assert_eq!(View::Forecast.first_selected(), 0);
        assert!(View::Time.pages() && !View::Forecast.pages());
    }

    #[test]
    fn the_memory_rows_show_dashes_without_data() {
        let rows = memory(&quiet());
        assert_eq!(rows[0].value, "—");
        assert_eq!(rows[0].said, "no reviewed cards");
        assert_eq!(rows[1].title, "Pass rate, last 30 days");
        assert_eq!(rows[1].said, "no reviews");
        assert_eq!(rows[2].value, "—");
        assert_eq!(rows[2].said, "no reviews");
    }

    #[test]
    fn the_memory_rows_show_percentages_and_time() {
        let mut d = quiet();
        d.stats.expected_retention = Some(0.912);
        d.stats.pass_rate = Some(0.874);
        d.stats.days[3] = day(10, 9, 3_600_000);
        d.stats.days[29] = day(2, 2, 12 * 60_000);
        let rows = memory(&d);
        assert_eq!(rows[0].value, "91%");
        assert_eq!(rows[1].value, "87%");
        assert_eq!(rows[2].value, "1 h 12 min");
        assert_eq!(rows[2].said, "1 h 12 min");
        assert_eq!(rows[2].title, "Time studied, last 30 days");
    }

    #[test]
    fn nothing_to_show_needs_no_history_no_schedule_and_nothing_due() {
        assert!(nothing_to_show(&quiet()));
        let mut d = quiet();
        d.stats.days[0] = day(1, 1, 5_000);
        assert!(!nothing_to_show(&d));
        let mut d = quiet();
        d.stats.scheduled_cards = 1;
        assert!(!nothing_to_show(&d));
        let mut d = quiet();
        d.stats.forecast[4] = 2;
        assert!(!nothing_to_show(&d));
        // Reviews long ago, of cards since reset: there are months to see.
        let mut d = quiet();
        d.first_review_at = Some(3);
        assert!(!nothing_to_show(&d));
    }

    #[test]
    fn paging_back_stops_at_the_period_of_the_first_review() {
        let mut d = quiet();
        // No review at all: no paging.
        assert!(!can_page_back(&d, 0));
        // The first review in this period (which starts at 100).
        d.first_review_at = Some(100);
        assert!(!can_page_back(&d, 0));
        d.first_review_at = Some(120);
        assert!(!can_page_back(&d, 3));
        // Before it: one more period back.
        d.first_review_at = Some(99);
        assert!(can_page_back(&d, 0));
        assert!(can_page_back(&d, 7));
        // Never past the furthest period.
        assert!(!can_page_back(&d, STATS_MAX_PERIOD));
        d.past_starts.clear();
        assert!(!can_page_back(&d, 0));
    }

    #[test]
    fn next_is_off_on_the_current_period_and_paging_stays_in_range() {
        assert!(!can_page_forward(0));
        assert!(can_page_forward(1));
        assert_eq!(paged(0, true), 1);
        assert_eq!(paged(3, false), 2);
        assert_eq!(paged(0, false), 0);
        assert_eq!(paged(STATS_MAX_PERIOD, true), STATS_MAX_PERIOD);
        assert_eq!(paged(u32::MAX, true), STATS_MAX_PERIOD);
    }

    #[test]
    fn the_reviews_figures_name_today_and_its_failed_reviews() {
        let mut days = vec![day(0, 0, 0); 30];
        days[29] = day(54, 51, 0);
        days[2] = day(1_200, 1_000, 0);
        let d = data(days, vec![0; 30]);
        let [today, period] = figures(View::Reviews, &d, 0, 29, &names());
        assert_eq!(today.key, "Today");
        assert_eq!(today.value, "54");
        assert_eq!(today.said, "54 reviews");
        assert_eq!(today.detail.as_deref(), Some("3 failed"));
        assert_eq!(period.key, "Last 30 Days");
        assert_eq!(period.value, "1,254");
        assert_eq!(period.detail, None);
        // Another day, and an earlier period: named by their dates.
        let [other, _] = figures(View::Reviews, &d, 0, 2, &names());
        assert_eq!(other.key, "Day 102");
        assert_eq!(other.detail.as_deref(), Some("200 failed"));
        let [last, period] = figures(View::Reviews, &d, 2, 29, &names());
        assert_eq!(last.key, "Day 129");
        assert_eq!(period.key, "d100 – d129");
        let [one, _] = figures(
            View::Reviews,
            &data(vec![day(1, 0, 0); 30], vec![0; 30]),
            0,
            0,
            &names(),
        );
        assert_eq!(one.said, "1 review");
        assert_eq!(one.detail.as_deref(), Some("1 failed"));
    }

    #[test]
    fn the_time_and_forecast_figures() {
        let mut days = vec![day(0, 0, 0); 30];
        days[29] = day(5, 5, 12 * 60_000);
        days[10] = day(5, 5, 60 * 60_000);
        let mut forecast = vec![0; 30];
        forecast[0] = 3;
        forecast[9] = 1;
        let d = data(days, forecast);
        let [today, period] = figures(View::Time, &d, 0, 29, &names());
        assert_eq!(
            (today.key.as_str(), today.value.as_str()),
            ("Today", "12 min")
        );
        assert_eq!(period.value, "1 h 12 min");
        let [tomorrow, next] = figures(View::Forecast, &d, 0, 0, &names());
        assert_eq!(tomorrow.key, "Tomorrow");
        assert_eq!(tomorrow.value, "3 cards");
        assert_eq!(tomorrow.said, "3 cards due");
        assert_eq!(next.key, "Next 30 Days");
        assert_eq!(next.value, "4 cards");
        let [later, _] = figures(View::Forecast, &d, 0, 9, &names());
        assert_eq!(
            (later.key.as_str(), later.value.as_str()),
            ("Day 1009", "1 card")
        );
    }

    #[test]
    fn the_reviews_chart_stacks_failed_on_passed_and_says_both() {
        let mut days = vec![day(0, 0, 0); 30];
        days[2] = day(38, 35, 0);
        days[9] = day(38, 30, 0);
        days[29] = day(5, 5, 0);
        let m = chart(View::Reviews, &data(days, vec![0; 30]), 0, &names());
        // 81 reviews, 70 passed, 11 failed; the earliest of the equal
        // busiest days.
        assert_eq!(
            m.summary,
            "Last 30 Days: 81 reviews, 70 passed, 11 failed, busiest day day 102 with 38."
        );
        assert_eq!(m.label, "Reviews per Day");
        assert_eq!(m.bars.len(), 30);
        assert_eq!(
            m.bars[2].caption,
            "Day 102: 38 reviews, 35 passed, 3 failed"
        );
        assert_eq!(
            m.bars[2].bar,
            Bar {
                lower: 35.0,
                upper: 3.0
            }
        );
        assert_eq!(m.bars[0].caption, "Day 100: no reviews");
        assert_eq!(m.bars[29].caption, "Today: 5 reviews, 5 passed, 0 failed");
        // Grid lines every 10 up to 40, labelled.
        let labels: Vec<&str> = m.ticks.iter().map(|t| t.label.as_str()).collect();
        assert_eq!(labels, ["0", "10", "20", "30", "40"]);
        assert_eq!(m.ceiling(), 40.0);
    }

    #[test]
    fn an_empty_period_is_said() {
        let m = chart(View::Reviews, &quiet(), 0, &names());
        assert_eq!(m.summary, "Last 30 Days: no reviews.");
        assert_eq!(m.ceiling(), 1.0);
        let m = chart(View::Time, &quiet(), 1, &names());
        assert_eq!(m.summary, "d100 – d129: no time studied.");
        let m = chart(View::Forecast, &quiet(), 0, &names());
        assert_eq!(m.summary, "Next 30 Days: nothing due.");
    }

    #[test]
    fn a_period_of_another_year_names_its_year() {
        let mut d = quiet();
        // This year: no year.
        assert_eq!(period_name(View::Reviews, &d, 1, &names()), "d100 – d129");
        // Starting in December 2025 while today is in 2026.
        d.past_starts = (90..120).collect();
        assert_eq!(period_name(View::Reviews, &d, 1, &names()), "d90y – d119y");
        // No forecast days to tell today by: as this year.
        d.ahead_starts.clear();
        assert_eq!(period_name(View::Reviews, &d, 1, &names()), "d90 – d119");
    }

    #[test]
    fn the_time_scale_turns_to_hours_above_two_hours() {
        let mut days = vec![day(0, 0, 0); 30];
        days[3] = day(10, 8, 150 * 60_000);
        days[4] = day(10, 8, 30 * 60_000);
        let m = chart(View::Time, &data(days, vec![0; 30]), 0, &names());
        assert_eq!(m.bars[3].bar.lower, 2.5);
        assert_eq!(m.bars[4].bar.lower, 0.5);
        let labels: Vec<&str> = m.ticks.iter().map(|t| t.label.as_str()).collect();
        assert_eq!(labels, ["0 h", "1 h", "2 h", "3 h"]);
        // The captions keep their own times.
        assert_eq!(m.bars[3].caption, "Day 103: 2 h 30 min studied");
    }

    #[test]
    fn the_time_chart_is_in_minutes() {
        let mut days = vec![day(0, 0, 0); 30];
        days[1] = day(10, 8, 90_000);
        days[7] = day(40, 30, 25 * 60_000);
        let m = chart(View::Time, &data(days, vec![0; 30]), 0, &names());
        assert_eq!(m.bars[7].bar.lower, 25.0);
        assert_eq!(m.bars[1].caption, "Day 101: 1 min studied");
        assert_eq!(m.bars[0].caption, "Day 100: no time studied");
        assert_eq!(
            m.summary,
            "Last 30 Days: 26 min studied, longest day day 107 with 25 min."
        );
        let labels: Vec<&str> = m.ticks.iter().map(|t| t.label.as_str()).collect();
        assert_eq!(labels, ["0 min", "10 min", "20 min", "30 min"]);
    }

    #[test]
    fn the_forecast_starts_tomorrow() {
        let mut forecast = vec![0; 30];
        forecast[0] = 12;
        forecast[5] = 40;
        let m = chart(
            View::Forecast,
            &data(vec![day(0, 0, 0); 30], forecast),
            0,
            &names(),
        );
        assert_eq!(
            m.summary,
            "Next 30 Days: 52 cards due, busiest day day 1005 with 40."
        );
        assert_eq!(m.bars[0].caption, "Tomorrow: 12 cards due");
        assert_eq!(m.bars[5].caption, "Day 1005: 40 cards due");
        assert_eq!(m.bars[1].caption, "Day 1001: nothing due");
        assert_eq!(m.dates[0].as_deref(), Some("d1000"));
        let mut forecast = vec![0; 30];
        forecast[0] = 1;
        let m = chart(
            View::Forecast,
            &data(vec![day(0, 0, 0); 30], forecast),
            0,
            &names(),
        );
        assert_eq!(
            m.summary,
            "Next 30 Days: 1 card due, busiest day tomorrow with 1."
        );
    }

    #[test]
    fn a_date_is_written_under_each_week_with_the_month_when_it_changes() {
        let starts: Vec<i64> = (100..130).collect();
        let dates = week_dates(&starts, &names());
        assert_eq!(dates.len(), 30);
        let written: Vec<(usize, &str)> = dates
            .iter()
            .enumerate()
            .filter_map(|(i, d)| d.as_deref().map(|d| (i, d)))
            .collect();
        // Day 114 starts month 2, so 114 is named in full.
        assert_eq!(
            written,
            [(0, "d100"), (7, "7"), (14, "d114"), (21, "21"), (28, "28")]
        );
        assert_eq!(week_dates(&[], &names()).len(), 0);
    }

    /// Counts and durations as large as the database can hold, negative
    /// durations and far dates: nothing panics or overflows, and every
    /// number stays sane (ADR 0050, rows 23 and 34).
    #[test]
    fn hostile_numbers_never_panic_and_stay_sane() {
        let hostile = [
            day(u32::MAX, u32::MAX, i64::MAX),
            day(u32::MAX, 0, i64::MIN),
            day(5, 9, -1),
        ];
        let mut days: Vec<DayActivity> = hostile.iter().copied().cycle().take(30).collect();
        days[0] = day(u32::MAX, u32::MAX, i64::MAX);
        let mut all = data(days, vec![u32::MAX; 30]);
        all.stats.pass_rate = Some(f64::NAN);
        all.stats.expected_retention = Some(f64::INFINITY);
        all.past_starts = vec![i64::MIN; 30];
        all.ahead_starts = vec![i64::MAX; 30];
        all.first_review_at = Some(i64::MIN);
        let names = DayNames {
            short: &crate::ui::time::day_short,
            with_weekday: &crate::ui::time::day_with_weekday,
            long: &crate::ui::time::day_long,
            short_with_year: &crate::ui::time::day_short_year,
            number: &crate::ui::time::day_number,
            month: &crate::ui::time::month_index,
        };
        for row in &memory(&all) {
            assert!(
                !row.value.contains("NaN") && !row.value.contains("inf"),
                "{row:?}"
            );
        }
        for view in [View::Reviews, View::Time, View::Forecast] {
            for period in [0, 1, STATS_MAX_PERIOD] {
                let chart = chart(view, &all, period, &names);
                assert!(chart.ceiling().is_finite() && chart.ceiling() >= 1.0);
                assert!(chart.ticks.len() <= 6);
                for bar in &chart.bars {
                    assert!(bar.bar.lower.is_finite() && bar.bar.lower >= 0.0);
                    assert!(bar.bar.upper.is_finite() && bar.bar.upper >= 0.0);
                }
                assert!(!chart.summary.contains('-'), "{}", chart.summary);
                assert!(chart.summary.len() < 300, "{}", chart.summary);
                for selected in [0, 29, 400] {
                    let _ = figures(view, &all, period, selected, &names);
                }
            }
        }
        assert!(
            chart(View::Reviews, &all, 0, &names)
                .summary
                .starts_with("Last 30 Days: 4,294,967,295 reviews")
        );
        assert!(!can_page_back(&all, 0));
    }

    #[test]
    fn the_texts_are_french_in_french() {
        crate::i18n::tests::in_french(|| {
            let mut days = vec![day(0, 0, 0); 30];
            days[29] = day(54, 51, 0);
            let d = data(days, vec![0; 30]);
            let [today, period] = figures(View::Reviews, &d, 0, 29, &names());
            assert_eq!(today.key, "Aujourd’hui");
            assert_eq!(today.detail.as_deref(), Some("3 ratées"));
            assert_eq!(period.key, "30 derniers jours");
            let [tomorrow, _] = figures(View::Forecast, &d, 0, 0, &names());
            assert_eq!(tomorrow.key, "Demain");
            // The chart counts reviews done; only the setting is a limit.
            assert_eq!(chart_title(View::Reviews), "Révisions par jour");
        });
    }
}
