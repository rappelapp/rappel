# 78. Statistics: design D, a month at a time

- Status: Accepted
- Date: 2026-11-09
- Amends: ADR 0033 (the statistics page), SPEC §4.7 (paging, a 30-day forecast)
- Refers to: Phase 3b brief, slice 4; mockup `docs/briefs/mockups/stats-mockups.html`, section `#d-month` (function `E()`), chosen by the maintainer; ADR 0032 (statistics in core), 0033 (the statistics screen it replaces), 0046 and 0058 (empty state, narrow and short windows), 0050 (hostile numbers); libadwaita 1.8 (`AdwToggleGroup`, `AdwWrapBox`), GTK 4.24 docs from `Gtk-4.0.gir` and `Adw-1.gir` cited in the code

## Context

The statistics page was three headline tiles over three charts, one per measure, each with "Up to N", hatched fails and "Select a day to see its values" (ADR 0033). It was a long page, and a phone showed one chart at a time. The maintainer chose design D of the mockups, "a month at a time": one card in the style of GNOME Wellbeing with a switch for Reviews, Time and Forecast, the figures of the selected day and of the period above 30 bars, round buttons that page 30 days at a time, and the three tiles as a "Memory" list under the card.

## Decision

**One card.** At the top of the 600 px clamp, a `card` box holds an `AdwToggleGroup` (Reviews, Time, Forecast; libadwaita ≥ 1.7, and the workspace already builds libadwaita with `v1_8`, so no fallback is needed), the two figures, the legend, the chart and the paging buttons. The page follows the group's `active-name`. Under the card, an `AdwPreferencesGroup` "Memory" has three `AdwActionRow`s with their value at the end: Expected retention, Pass rate (last 30 days), Time studied (last 30 days). They always describe the current period: paging changes only the chart and its figures.

**Figures, as Wellbeing.** Two figures in an `AdwWrapBox` (the second wraps under the first if a line cannot hold both): the selected day ("Today", "Tomorrow" or "Tue 14 Oct") and the period ("Last 30 Days", "8 Sep – 7 Oct" for an earlier period, with the year when it is not this year ("8 Dec 2025 – 6 Jan 2026"), "Next 30 Days" for the forecast). Reviews shows the count and, under the day's count, "N failed"; Time shows times; Forecast shows "N cards". Each value's accessible label says it in words ("54 reviews"). The legend (Passed, Failed, with a context of their own so a language can use the plural) is at the top right on Reviews only, and under the figures below 500 sp, where a 360 px window cannot hold it beside them.

**The chart.** `BarChart` keeps its `GtkDrawingArea` and Cairo for the shapes, and now draws texts too: in its `snapshot`, after the drawing area's own, Pango layouts made by the widget (so in its font and text size; the chart's height follows a change of the font, its DPI or the font configuration (`system_setting_changed`), at libadwaita's caption scale with tabular figures) give each grid line's value at the right and a date under every seventh day ("11 Oct", "18", "25", "1 Nov" when the month changes). Grid steps are 1, 2 or 5 times a power of ten (on Time, minutes up to two hours, then whole hours), at least 1, with at most four lines above the base (`layout::ticks`). Bars are the accent colour with rounded tops (4 px at most). Fails are stacked on passes in the same accent at 38 % above a 2 px gap, with their own rounded top and at least 3 px tall; there is no hatching and no average line. The selected day has a light rounded highlight (the text colour at 10 %). Colours are only the accent colour from `AdwStyleManager` and the widget's CSS colour at lower opacities, so light, dark and accent changes need nothing of ours.

**Failed is never colour alone.** Besides the gap and the shape, the chart's accessible description says the period's failed count ("Last 30 Days: 567 reviews, 500 passed, 67 failed, busiest day …"), and the caption announced for a selected day says that day's ("Tue 20 Oct: 36 reviews, 32 passed, 4 failed"), as the "N failed" figure does on screen.

**Selecting a day.** Today is selected at first (tomorrow on Forecast), and after paging the last day of the new period. Left, Right, Home and End move the selection as before. At 360 px the bars are about 7 px wide (`BAR_SHARE` 0.72 of a slot of about 9.5 px), so a press anywhere on the chart, the values and dates included, selects the day nearest to it horizontally (`layout::day_near`): the whole height of a slot is its hit area, and a press beyond the bars picks the end day. The day is selected when the press is released, so that a swipe starting on the chart, whose touch the scrolled window claims, should scroll without selecting (not yet tried on a real touch screen). Reviews and Time share their selected day; the forecast keeps its own.

**Paging.** Previous and Next go 30 days back or forward; Next is insensitive on the current period. Previous is sensitive only while the first review of the scope is before the shown period's first day (`text::can_page_back`), so paging stops at the period holding the first review and never runs on through empty months; without any review it is off. Periods are capped at 1,000 (about 82 years), so day offsets stay small whatever an import's dates. A paging button that turns insensitive while focused hands the focus to the other one (or the chart). The forecast does not page: it is always the next 30 days, and the buttons are hidden on Forecast rather than shown and always off.

**Data.** `AppState::stats(deck, now, period)` reads one period with the clock of the moment the page opened (the days stay put while it is open) and the forecast of the 30 days from tomorrow. Core's forecast puts every overdue card on its first day, so the app asks for 31 days from today and drops today, leaving each day from tomorrow with only what falls due on it. One small core addition, `Store::first_activity_at(scope)` (and `Store::stats_and_first_activity`, which reads it in the same transaction as the statistics, as the page does), gives the time of the earliest review the activity counts (surviving reviews of live cards in live decks, archived decks too for every deck), with the same SQL rule as the window's; its property test compares it with the pure rule over the logs, and it is timed at 100,000 reviews. No migration and no export change. Paging reads the database again on the main thread; at 5,000 cards and 100,000 reviews the read is the one ADR 0032 measured. A read that fails leaves the page as it was and shows an error toast.

**The empty state** is unchanged, and now also needs no review ever: a collection whose only reviews are older than 30 days has periods to page back to.

**Not done**: the mockup's secondary "Average" figures (the brief's bullets do not ask for them).

**Removed**: `ChartCard` and `data/chart_card.blp` (the per-chart card), the "Up to N" scale, the hint "Select a day to see its values", the empty sentences under a chart (an empty period shows its axis and the summary says "no reviews"), and the forecast's "N overdue cards are counted under Today" note, since the forecast starts tomorrow; overdue and today's due cards are what the deck list's due counts show.

**Scratch data.** `fill_scratch_db --history DAYS` adds a "History" deck answered on most of the past DAYS days, to try paging.

Alternatives not taken:

- **Design D's week variant, or A, B, C**: the maintainer chose the month.
- **Paging the forecast forward**: it would need the latest due date to stop, and the next 30 days answer the question the forecast is for.
- **Previous always sensitive, with empty periods**: paging "forever" through months with nothing in them.
- **A forecast from today with overdue cards under Today** (as before): the brief's figures are Tomorrow and Next 30 Days, as in the mockup.
- **Drag to scrub across days by touch**: a horizontal drag in a vertically scrolling page can fight the scrolled window's own touch gesture; the nearest-day press already gives each day a hit area as tall as the chart.
- **Labels as GTK widgets aligned to the grid**: they cannot follow the grid lines exactly; Pango layouts in the chart's own snapshot can, and still use the widget's font.
