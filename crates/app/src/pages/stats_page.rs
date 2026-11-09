//! The statistics page, design D "a month at a time" (SPEC §4.7, ADR 0032,
//! 0033, 0078): one card with a switch for Reviews, Time and Forecast, the
//! selected day's and the period's figures, a chart of 30 days and buttons
//! that page back and forth 30 days at a time; under it, the Memory list.
//! Layout in `data/stats_page.blp`; every text, number and paging rule is
//! decided in the pure [`text`] module.
//!
//! The current period is read when the page is built, with the clock of
//! that moment; paging reads another period with the same clock, so the
//! days stay put while the page is open. Reopening the page reads again.

mod text;

use crate::i18n::gettext;
use adw::prelude::*;
use adw::subclass::prelude::*;
use gtk::glib;
use rappel_core::domain::Deck;

use self::text::{DayNames, View};
use crate::state::{AppState, StatsData};
use crate::ui::time;
use crate::widgets::bar_chart::{StyleWatch, paint_swatch};

/// The empty state's icon, as in its layout, given back after a short
/// window took it off (`ui::breakpoint::compact_status_page`).
const EMPTY_ICON: &str = "document-open-recent-symbolic";

/// How days are named on this page, in the local zone and language.
fn day_names() -> DayNames<'static> {
    DayNames {
        short: &time::day_short,
        with_weekday: &time::day_with_weekday,
        long: &time::day_long,
        short_with_year: &time::day_short_year,
        number: &time::day_number,
        month: &time::month_index,
    }
}

mod imp {
    use std::cell::{Cell, RefCell};

    use adw::prelude::*;
    use adw::subclass::prelude::*;
    use gtk::glib;
    use gtk::glib::prelude::StaticTypeExt;

    use crate::state::{AppState, StatsData};
    use crate::widgets::bar_chart::{BarChart, StyleWatch};

    #[derive(Debug, Default, gtk::CompositeTemplate)]
    #[template(resource = "/io/github/rappelapp/Rappel/stats_page.ui")]
    pub struct StatsPage {
        #[template_child]
        pub empty_short: TemplateChild<adw::Breakpoint>,
        #[template_child]
        pub empty_status: TemplateChild<adw::StatusPage>,
        #[template_child]
        pub window_title: TemplateChild<adw::WindowTitle>,
        #[template_child]
        pub content_stack: TemplateChild<gtk::Stack>,
        #[template_child]
        pub back_button: TemplateChild<gtk::Button>,
        #[template_child]
        pub view_toggles: TemplateChild<adw::ToggleGroup>,
        #[template_child]
        pub day_key: TemplateChild<gtk::Label>,
        #[template_child]
        pub day_value: TemplateChild<gtk::Label>,
        #[template_child]
        pub day_detail: TemplateChild<gtk::Label>,
        #[template_child]
        pub period_key: TemplateChild<gtk::Label>,
        #[template_child]
        pub period_value: TemplateChild<gtk::Label>,
        #[template_child]
        pub legend_box: TemplateChild<gtk::Box>,
        #[template_child]
        pub passed_swatch: TemplateChild<gtk::DrawingArea>,
        #[template_child]
        pub failed_swatch: TemplateChild<gtk::DrawingArea>,
        #[template_child]
        pub chart: TemplateChild<BarChart>,
        #[template_child]
        pub nav_box: TemplateChild<gtk::Box>,
        #[template_child]
        pub previous_button: TemplateChild<gtk::Button>,
        #[template_child]
        pub next_button: TemplateChild<gtk::Button>,
        #[template_child]
        pub retention_row: TemplateChild<adw::ActionRow>,
        #[template_child]
        pub pass_rate_row: TemplateChild<adw::ActionRow>,
        #[template_child]
        pub time_row: TemplateChild<adw::ActionRow>,
        #[template_child]
        pub retention_value: TemplateChild<gtk::Label>,
        #[template_child]
        pub pass_rate_value: TemplateChild<gtk::Label>,
        #[template_child]
        pub time_value: TemplateChild<gtk::Label>,
        /// Where the data comes from: the app state, the deck (or `None`
        /// for every deck) and the clock the page was opened with.
        pub source: RefCell<Option<(AppState, Option<String>, i64)>>,
        /// The current period (0), for the Memory rows and the forecast.
        pub current: RefCell<Option<StatsData>>,
        /// The period shown by Reviews and Time, and its number.
        pub shown: RefCell<Option<StatsData>>,
        pub period: Cell<u32>,
        /// The selected day of the period, and of the forecast.
        pub past_day: Cell<usize>,
        pub ahead_day: Cell<usize>,
        pub style_watch: RefCell<Vec<StyleWatch>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for StatsPage {
        const NAME: &'static str = "RappelStatsPage";
        type Type = super::StatsPage;
        type ParentType = adw::NavigationPage;

        fn class_init(klass: &mut Self::Class) {
            // The template names this type.
            BarChart::ensure_type();
            klass.bind_template();
        }

        fn instance_init(obj: &glib::subclass::InitializingObject<Self>) {
            obj.init_template();
        }
    }

    impl ObjectImpl for StatsPage {
        fn constructed(&self) {
            self.parent_constructed();
            // The empty state on a short window (ADR 0058).
            crate::ui::breakpoint::compact_status_page(
                &self.empty_short,
                &self.empty_status,
                super::EMPTY_ICON,
            );
            // GTK documents `shown` as the place to set the focus after a
            // push (ui/nav.rs): earlier, the view focuses the Back button.
            self.obj().connect_shown(super::StatsPage::focus_chart);
            self.obj().connect_controls();
        }
    }
    impl WidgetImpl for StatsPage {}
    impl NavigationPageImpl for StatsPage {}
}

glib::wrapper! {
    pub struct StatsPage(ObjectSubclass<imp::StatsPage>)
        @extends adw::NavigationPage, gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}

impl StatsPage {
    /// The page of `deck`, or of every deck for `None`, as of now.
    ///
    /// # Errors
    /// If the statistics cannot be read.
    pub fn new(state: &AppState, deck: Option<&Deck>) -> rappel_core::Result<Self> {
        let now = rappel_core::time::now_ms();
        let deck_id = deck.map(|d| d.id.clone());
        let data = state.stats(deck_id.as_deref(), now, 0)?;
        let page: Self = glib::Object::new();
        let imp = page.imp();
        // Deck names are untrusted; AdwWindowTitle shows plain text.
        imp.window_title
            .set_subtitle(&deck.map_or_else(|| gettext("All Decks"), |d| d.name.clone()));
        imp.back_button.set_label(&text::back_label(deck.is_some()));
        imp.source.replace(Some((state.clone(), deck_id, now)));
        page.fill(data);
        Ok(page)
    }

    /// Wires the switch, the chart, the paging buttons and the legend.
    fn connect_controls(&self) {
        let imp = self.imp();
        // `active-name` follows the active toggle, however it changes
        // (Adw-1.gir, `AdwToggleGroup:active-name`).
        imp.view_toggles.connect_active_name_notify(glib::clone!(
            #[weak(rename_to = page)]
            self,
            move |_| page.refresh()
        ));
        imp.chart.connect_selected(glib::clone!(
            #[weak(rename_to = page)]
            self,
            move |day| {
                let imp = page.imp();
                if page.view().pages() {
                    imp.past_day.set(day);
                } else {
                    imp.ahead_day.set(day);
                }
                page.show_figures();
            }
        ));
        imp.previous_button.connect_clicked(glib::clone!(
            #[weak(rename_to = page)]
            self,
            move |_| page.page(true)
        ));
        imp.next_button.connect_clicked(glib::clone!(
            #[weak(rename_to = page)]
            self,
            move |_| page.page(false)
        ));
        for (swatch, upper) in [(&*imp.passed_swatch, false), (&*imp.failed_swatch, true)] {
            swatch.set_draw_func(move |_, cr, width, height| {
                paint_swatch(cr, width, height, upper);
            });
            swatch.update_state(&[gtk::accessible::State::Hidden(true)]);
        }
        imp.style_watch.replace(vec![
            StyleWatch::new(&*imp.passed_swatch),
            StyleWatch::new(&*imp.failed_swatch),
        ]);
        imp.past_day.set(View::Reviews.first_selected());
        imp.ahead_day.set(View::Forecast.first_selected());
    }

    fn fill(&self, data: StatsData) {
        let imp = self.imp();
        if text::nothing_to_show(&data) {
            imp.content_stack.set_visible_child_name("empty");
            return;
        }
        imp.content_stack.set_visible_child_name("stats");
        let rows = text::memory(&data);
        let values = [
            (&imp.retention_row, &imp.retention_value),
            (&imp.pass_rate_row, &imp.pass_rate_value),
            (&imp.time_row, &imp.time_value),
        ];
        for ((action_row, label), row) in values.into_iter().zip(&rows) {
            action_row.set_title(&row.title);
            label.set_label(&row.value);
            // "—" is read as words ("no reviews").
            label.update_property(&[gtk::accessible::Property::Label(&row.said)]);
        }
        imp.current.replace(Some(data.clone()));
        imp.shown.replace(Some(data));
        imp.period.set(0);
        self.refresh();
    }

    /// The view the switch shows.
    fn view(&self) -> View {
        View::from_name(self.imp().view_toggles.active_name().as_deref())
    }

    /// Shows the view's chart, figures, legend and paging buttons.
    fn refresh(&self) {
        let imp = self.imp();
        let view = self.view();
        let period = imp.period.get();
        let model = {
            let shown = imp.shown.borrow();
            let current = imp.current.borrow();
            // The forecast is the same in every period; the current one's.
            let data = if view.pages() { &*shown } else { &*current };
            let Some(data) = data.as_ref() else {
                return;
            };
            text::chart(view, data, period, &day_names())
        };
        let selected = if view.pages() {
            imp.past_day.get()
        } else {
            imp.ahead_day.get()
        };
        imp.chart.set_model(model, Some(selected));
        imp.legend_box.set_visible(view == View::Reviews);
        // Hiding a focused paging button would lose the keyboard focus.
        let focused = imp.previous_button.has_focus() || imp.next_button.has_focus();
        imp.nav_box.set_visible(view.pages());
        if focused && !view.pages() {
            imp.chart.grab_focus();
        }
        self.update_paging();
        self.show_figures();
    }

    /// Makes Previous and Next sensitive or not for the shown period. A
    /// button that loses its sensitivity while focused hands the focus to
    /// the other one, so the keyboard focus is never lost.
    fn update_paging(&self) {
        let imp = self.imp();
        let period = imp.period.get();
        let back = imp
            .shown
            .borrow()
            .as_ref()
            .is_some_and(|data| text::can_page_back(data, period));
        let forward = text::can_page_forward(period);
        let pairs = [
            (&*imp.previous_button, back, &*imp.next_button),
            (&*imp.next_button, forward, &*imp.previous_button),
        ];
        let lost_focus = pairs
            .iter()
            .find(|(button, on, _)| !on && button.has_focus())
            .map(|(_, _, other)| *other);
        for (button, on, _) in &pairs {
            button.set_sensitive(*on);
        }
        if let Some(other) = lost_focus {
            if other.is_sensitive() {
                other.grab_focus();
            } else {
                imp.chart.grab_focus();
            }
        }
    }

    /// Shows the selected day's and the period's figures.
    fn show_figures(&self) {
        let imp = self.imp();
        let view = self.view();
        let period = imp.period.get();
        let figures = {
            let shown = imp.shown.borrow();
            let current = imp.current.borrow();
            let data = if view.pages() { &*shown } else { &*current };
            let Some(data) = data.as_ref() else {
                return;
            };
            let selected = imp
                .chart
                .selected()
                .unwrap_or_else(|| view.first_selected());
            text::figures(view, data, period, selected, &day_names())
        };
        let [day, whole] = figures;
        imp.day_key.set_label(&day.key);
        imp.day_value.set_label(&day.value);
        imp.day_value
            .update_property(&[gtk::accessible::Property::Label(&day.said)]);
        imp.day_detail
            .set_label(day.detail.as_deref().unwrap_or_default());
        imp.day_detail.set_visible(day.detail.is_some());
        imp.period_key.set_label(&whole.key);
        imp.period_value.set_label(&whole.value);
        imp.period_value
            .update_property(&[gtk::accessible::Property::Label(&whole.said)]);
    }

    /// Pages back (`back`) or forward one period, reading it; the last day
    /// of the new period is selected. On a failed read the page stays as
    /// it was and says so.
    fn page(&self, back: bool) {
        let imp = self.imp();
        let period = text::paged(imp.period.get(), back);
        let read = imp
            .source
            .borrow()
            .as_ref()
            .map(|(state, deck, now)| state.stats(deck.as_deref(), *now, period));
        match read {
            Some(Ok(data)) => {
                imp.shown.replace(Some(data));
                imp.period.set(period);
                imp.past_day.set(View::Reviews.first_selected());
                self.refresh();
            }
            Some(Err(err)) => crate::ui::toast::show_error(
                self,
                "reading statistics",
                &crate::ui::text::with_error(
                    // Translators: a toast: what is before ":" is its title,
                    // the rest is behind Details. {error} is why, in English.
                    &gettext("The statistics could not be read: {error}"),
                    &err,
                ),
            ),
            None => {}
        }
    }

    /// Puts the keyboard focus on the chart, or on the empty state's
    /// button.
    fn focus_chart(&self) {
        let imp = self.imp();
        if imp.content_stack.visible_child_name().as_deref() == Some("empty") {
            imp.back_button.grab_focus();
        } else {
            imp.chart.grab_focus();
        }
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use rappel_core::domain::Grade;

    use super::*;

    /// A deck of two cards, each passed twice ten days ago and
    /// failed once yesterday.
    fn studied(state: &AppState) -> Deck {
        let deck = state.create_deck("Deck").unwrap();
        let now = rappel_core::time::now_ms();
        let day = crate::ui::time::DAY_MS;
        for i in 0..2 {
            let id = state
                .add_card_with_tags(&deck.id, &format!("front {i}"), "back", &[])
                .unwrap()
                .card
                .id;
            for (grade, at) in [
                (Grade::Pass, now - 10 * day),
                (Grade::Pass, now - 10 * day + 660_000),
                (Grade::Fail, now - day),
            ] {
                state
                    .record_review(&id, grade, Duration::from_secs(5), at)
                    .unwrap();
            }
        }
        deck
    }

    fn shown(page: &StatsPage) -> Option<String> {
        page.imp()
            .content_stack
            .visible_child_name()
            .map(|n| n.to_string())
    }

    #[test]
    fn nothing_studied_shows_the_empty_state() {
        crate::testing::with_widgets(|| {
            let (_tmp, state) = crate::testing::temporary_state();
            let page = StatsPage::new(&state, None).unwrap();
            assert_eq!(shown(&page).as_deref(), Some("empty"));
            assert_eq!(
                page.imp().back_button.label().as_deref(),
                Some("_Back to Decks")
            );
            assert_eq!(
                page.imp().back_button.action_name().as_deref(),
                Some("navigation.pop")
            );
            // A deck of new cards has nothing to show either.
            let deck = state.create_deck("Deck").unwrap();
            state
                .add_card_with_tags(&deck.id, "front", "back", &[])
                .unwrap();
            let page = StatsPage::new(&state, Some(&deck)).unwrap();
            assert_eq!(shown(&page).as_deref(), Some("empty"));
            assert_eq!(
                page.imp().back_button.label().as_deref(),
                Some("_Back to Deck")
            );
        });
    }

    #[test]
    fn studied_cards_fill_the_figures_the_chart_and_the_memory_rows() {
        crate::testing::with_widgets(|| {
            let (_tmp, state) = crate::testing::temporary_state();
            let deck = studied(&state);
            let page = StatsPage::new(&state, Some(&deck)).unwrap();
            let imp = page.imp();
            assert_eq!(shown(&page).as_deref(), Some("stats"));
            assert_eq!(imp.window_title.subtitle(), "Deck");
            assert_ne!(imp.retention_value.label(), "—");
            // Six answers in the 30 days, two failed.
            assert_eq!(imp.pass_rate_value.label(), "67%");
            assert_eq!(imp.time_value.label(), "30 s");
            assert_eq!(imp.pass_rate_row.title(), "Pass rate, last 30 days");
            // Reviews, today selected; the period's total beside it.
            assert_eq!(imp.chart.selected(), Some(29));
            assert_eq!(imp.day_key.label(), "Today");
            assert_eq!(imp.period_key.label(), "Last 30 Days");
            assert_eq!(imp.period_value.label(), "6");
            assert!(imp.legend_box.is_visible());
            assert!(imp.nav_box.is_visible());
            assert!(!imp.next_button.is_sensitive());
            // All within this period: no paging back.
            assert!(!imp.previous_button.is_sensitive());
            // Yesterday: two reviews, both failed.
            imp.chart.select(28);
            assert_eq!(imp.day_value.label(), "2");
            assert_eq!(imp.day_detail.label(), "2 failed");
            assert!(imp.day_detail.is_visible());

            // Time: no legend, no "failed".
            imp.view_toggles.set_active_name(Some("time"));
            assert!(!imp.legend_box.is_visible());
            assert!(!imp.day_detail.is_visible());
            assert_eq!(imp.chart.selected(), Some(28));
            assert_eq!(imp.period_value.label(), "30 s");

            // Forecast: tomorrow selected, no paging.
            imp.view_toggles.set_active_name(Some("forecast"));
            assert_eq!(imp.chart.selected(), Some(0));
            assert_eq!(imp.day_key.label(), "Tomorrow");
            assert_eq!(imp.period_key.label(), "Next 30 Days");
            assert!(!imp.nav_box.is_visible());

            let all = StatsPage::new(&state, None).unwrap();
            assert_eq!(all.imp().window_title.subtitle(), "All Decks");
            assert_eq!(all.imp().pass_rate_value.label(), "67%");
        });
    }

    #[test]
    fn paging_goes_back_to_the_first_review_and_no_further() {
        crate::testing::with_widgets(|| {
            let (_tmp, state) = crate::testing::temporary_state();
            let deck = state.create_deck("Deck").unwrap();
            let id = state
                .add_card_with_tags(&deck.id, "front", "back", &[])
                .unwrap()
                .card
                .id;
            // Reviewed once, 45 days ago and failed: in the period before.
            let long_ago = rappel_core::time::now_ms() - 45 * crate::ui::time::DAY_MS;
            state
                .record_review(&id, Grade::Fail, Duration::from_secs(3), long_ago)
                .unwrap();
            let page = StatsPage::new(&state, None).unwrap();
            let imp = page.imp();
            assert_eq!(shown(&page).as_deref(), Some("stats"));
            assert_eq!(imp.period_value.label(), "0");
            assert_eq!(imp.pass_rate_value.label(), "—");
            assert_eq!(imp.time_value.label(), "—");
            assert!(imp.previous_button.is_sensitive());
            assert!(!imp.next_button.is_sensitive());

            imp.previous_button.emit_clicked();
            assert_eq!(imp.period.get(), 1);
            assert_eq!(imp.period_value.label(), "1");
            assert_ne!(imp.period_key.label(), "Last 30 Days");
            assert_ne!(imp.day_key.label(), "Today");
            // The first review is in this period: Previous is off now.
            assert!(!imp.previous_button.is_sensitive());
            assert!(imp.next_button.is_sensitive());
            // The Memory rows stay those of the last 30 days.
            assert_eq!(imp.pass_rate_value.label(), "—");

            imp.next_button.emit_clicked();
            assert_eq!(imp.period.get(), 0);
            assert_eq!(imp.period_key.label(), "Last 30 Days");
            assert_eq!(imp.day_key.label(), "Today");
            assert!(!imp.next_button.is_sensitive());
        });
    }

    #[test]
    fn a_deleted_deck_has_no_page() {
        crate::testing::with_widgets(|| {
            let (_tmp, state) = crate::testing::temporary_state();
            let deck = studied(&state);
            state.delete_deck(&deck.id).unwrap();
            assert!(matches!(
                StatsPage::new(&state, Some(&deck)),
                Err(rappel_core::Error::DeckNotFound(_))
            ));
        });
    }
}
