//! Preferences: the daily limits, the day start hour, the desired retention,
//! a button for the backup folder (SPEC §4.8, ADR 0027), the automatic
//! backup folder (ADR 0080) and Clean Up Media (`clean_up`, ADR 0057). Layout in
//! `data/preferences_dialog.blp` and `data/retention_dialog.blp`.
//!
//! The limits and the day start are saved as soon as they change. The
//! desired retention is asked about first, because changing it rebuilds
//! every card's schedule (an `AdwAlertDialog`), and that runs on a worker
//! thread, since a long review log takes a while to replay.

use std::cell::Cell;
use std::path::Path;
use std::rc::Rc;

use adw::prelude::*;
use gtk::{gio, glib};
use rappel_core::data_dir::ensure_data_dir;
use rappel_core::domain::{DESIRED_RETENTION_RANGE, MAX_DAILY_LIMIT, MAX_DAY_START_HOUR, Settings};

use crate::auto_backup;
use crate::dialogs::clean_up;
use crate::i18n::gettext;
use crate::state::AppState;
use crate::ui::dialog;
use crate::ui::text::with_error;
use crate::ui::toast::{ErrorSlot, log_failure, show_error};
use crate::ui::worker;
use crate::window::RappelWindow;

const PREFERENCES_FILE: &str = "preferences_dialog.ui";
const RETENTION_FILE: &str = "retention_dialog.ui";

/// The desired retention as a whole percent, for the spin rows and the
/// value shown ("90%").
pub fn percent_from_retention(retention: f32) -> u8 {
    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "a valid retention is between 0.80 and 0.97, so the rounded percent fits a u8; \
                  anything else is clamped first"
    )]
    let percent = (retention * 100.0).round().clamp(0.0, 100.0) as u8;
    percent
}

/// The desired retention for a whole `percent`, as the store keeps it: the
/// nearest `f32` to `percent / 100` (division is correctly rounded), which
/// is the same `f32` as the literal `0.90` for 90.
pub fn retention_from_percent(percent: u8) -> f32 {
    f32::from(percent) / 100.0
}

/// The text of the retention row's value, written like every other percent
/// in the app (`ui::text::percent`).
pub fn retention_label(retention: f32) -> String {
    crate::ui::text::whole_percent(percent_from_retention(retention))
}

/// A daily limit from a spin row's value.
pub fn limit_from_value(value: f64) -> u32 {
    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "the value is rounded and clamped to 0..=MAX_DAILY_LIMIT first"
    )]
    let limit = value.round().clamp(0.0, f64::from(MAX_DAILY_LIMIT)) as u32;
    limit
}

/// A percent from a spin row's value.
fn percent_from_value(value: f64) -> u8 {
    let (low, high) = (
        percent_from_retention(*DESIRED_RETENTION_RANGE.start()),
        percent_from_retention(*DESIRED_RETENTION_RANGE.end()),
    );
    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "the value is rounded and clamped to the retention range in percent first"
    )]
    // `max` and `min` return the other operand for NaN; `clamp` would keep it.
    let percent = value.round().max(f64::from(low)).min(f64::from(high)) as u8;
    percent
}

/// The label of a day start hour in the combo row: "04:00".
pub fn hour_label(hour: u8) -> String {
    format!("{hour:02}:00")
}

/// The hour of combo row position `position`, or `None` past the last.
pub fn hour_from_position(position: u32) -> Option<u8> {
    u8::try_from(position)
        .ok()
        .filter(|&hour| hour <= MAX_DAY_START_HOUR)
}

/// What the dialog's handlers share. Holds no widget, so a handler owning
/// it cannot keep the dialog alive.
#[derive(Clone)]
struct Context {
    state: AppState,
    /// Something was saved, so the decks must be read again on closing.
    changed: Rc<Cell<bool>>,
    /// The rows are being filled in: their change handlers must not save.
    loading: Rc<Cell<bool>>,
    /// Shows the stored settings in the rows again.
    refresh: Rc<dyn Fn()>,
    /// The retention question or its rebuild is under way.
    retention_busy: Rc<Cell<bool>>,
    /// The error toast shown last; a new one replaces it.
    errors: Rc<ErrorSlot>,
}

/// The widgets of `preferences_dialog.blp` this file uses.
pub(crate) struct Rows {
    dialog: adw::PreferencesDialog,
    page: adw::PreferencesPage,
    new_per_day: adw::SpinRow,
    reviews_per_day: adw::SpinRow,
    day_start: adw::ComboRow,
    retention_row: adw::ActionRow,
    retention_value: gtk::Label,
    open_backups: adw::ButtonRow,
    auto_backup_row: adw::ActionRow,
    auto_backup_choose: gtk::Button,
    auto_backup_off: gtk::Button,
    clean_up_row: adw::ActionRow,
    clean_up_button: gtk::Button,
}

impl Rows {
    /// Loads the layout; `Err` with the first missing object's id, after
    /// logging every missing one.
    pub(crate) fn load() -> Result<Self, &'static str> {
        let b = dialog::builder(PREFERENCES_FILE);
        // Every object is looked up before the first `?`, so each missing
        // one is logged.
        let (dialog, page, new_per_day, reviews_per_day) = (
            object(&b, "preferences_dialog"),
            object(&b, "settings_page"),
            object(&b, "new_per_day"),
            object(&b, "reviews_per_day"),
        );
        let (day_start, retention_row, retention_value, open_backups) = (
            object(&b, "day_start"),
            object(&b, "retention_row"),
            object(&b, "retention_value"),
            object(&b, "open_backups"),
        );
        let (auto_backup_row, auto_backup_choose, auto_backup_off) = (
            object(&b, "auto_backup_row"),
            object(&b, "auto_backup_choose"),
            object(&b, "auto_backup_off"),
        );
        let (clean_up_row, clean_up_button) =
            (object(&b, "clean_up_row"), object(&b, "clean_up_button"));
        let rows = Self {
            dialog: dialog?,
            page: page?,
            new_per_day: new_per_day?,
            reviews_per_day: reviews_per_day?,
            day_start: day_start?,
            retention_row: retention_row?,
            retention_value: retention_value?,
            open_backups: open_backups?,
            auto_backup_row: auto_backup_row?,
            auto_backup_choose: auto_backup_choose?,
            auto_backup_off: auto_backup_off?,
            clean_up_row: clean_up_row?,
            clean_up_button: clean_up_button?,
        };
        let hours: Vec<String> = (0..=MAX_DAY_START_HOUR).map(hour_label).collect();
        let hours: Vec<&str> = hours.iter().map(String::as_str).collect();
        rows.day_start
            .set_model(Some(&gtk::StringList::new(&hours)));
        Ok(rows)
    }

    /// Shows `settings` in the rows without saving them again.
    fn fill(&self, loading: &Cell<bool>, settings: &Settings) {
        loading.set(true);
        self.new_per_day
            .set_value(f64::from(settings.max_new_per_day));
        self.reviews_per_day
            .set_value(f64::from(settings.max_reviews_per_day));
        self.day_start
            .set_selected(u32::from(settings.day_start_hour));
        self.retention_value
            .set_label(&retention_label(settings.desired_retention));
        loading.set(false);
    }
}

fn object<T: IsA<glib::Object>>(
    builder: &gtk::Builder,
    id: &'static str,
) -> Result<T, &'static str> {
    dialog::object(builder, PREFERENCES_FILE, id)
}

/// The retention question and its spin row, from
/// `data/retention_dialog.blp`; `Err` with the first missing object's id,
/// after logging every missing one.
pub(crate) fn retention_layout() -> Result<(adw::AlertDialog, adw::SpinRow), &'static str> {
    let builder = dialog::builder(RETENTION_FILE);
    let (alert, spin) = (
        dialog::object(&builder, RETENTION_FILE, "retention_dialog"),
        dialog::object(&builder, RETENTION_FILE, "retention_spin"),
    );
    Ok((alert?, spin?))
}

/// Shows the preferences over `window`, unless they are already open.
pub fn show(window: &RappelWindow, state: &AppState) {
    let open = window.dialogs();
    if (0..open.n_items()).any(|i| {
        open.item(i)
            .is_some_and(|d| d.is::<adw::PreferencesDialog>())
    }) {
        return;
    }
    let settings = match state.settings() {
        Ok(settings) => settings,
        Err(err) => {
            show_error(
                window,
                "reading the settings",
                // Translators: a toast: what is before ":" is its title, the rest is behind Details. {error} is why, in English.
                &with_error(&gettext("The settings could not be read: {error}"), &err),
            );
            return;
        }
    };
    let Ok(rows) = Rows::load() else {
        return;
    };

    let ctx = context(&rows, state);
    // Filled before any handler is connected, and under the guard anyway.
    rows.fill(&ctx.loading, &settings);

    connect_study_rows(&rows, &ctx);
    connect_retention(window, &rows, &ctx);
    connect_backups(window, &rows, &ctx);
    show_auto_backup(
        &rows.auto_backup_row,
        &rows.auto_backup_off,
        auto_backup::load(state.data_dir()).folder.as_deref(),
    );
    connect_auto_backup(window, &rows, &ctx);
    clean_up::connect(
        &rows.dialog,
        &rows.clean_up_row,
        &rows.clean_up_button,
        state,
        &ctx.errors,
    );

    // What the limits and the day start changed shows on the pages behind.
    let changed = ctx.changed.clone();
    rows.dialog.connect_closed(glib::clone!(
        #[weak]
        window,
        move |_| {
            if changed.get() {
                window.data_changed();
            }
        }
    ));
    rows.dialog.present(Some(window));
}

/// The state the handlers share. Its `refresh` holds the rows weakly.
fn context(rows: &Rows, state: &AppState) -> Context {
    let loading = Rc::new(Cell::new(false));
    let refresh: Rc<dyn Fn()> = Rc::new(glib::clone!(
        #[weak(rename_to = new_per_day)]
        rows.new_per_day,
        #[weak(rename_to = reviews_per_day)]
        rows.reviews_per_day,
        #[weak(rename_to = day_start)]
        rows.day_start,
        #[weak(rename_to = retention_value)]
        rows.retention_value,
        #[strong]
        state,
        #[strong]
        loading,
        move || match state.settings() {
            Ok(settings) => {
                loading.set(true);
                new_per_day.set_value(f64::from(settings.max_new_per_day));
                reviews_per_day.set_value(f64::from(settings.max_reviews_per_day));
                day_start.set_selected(u32::from(settings.day_start_hour));
                retention_value.set_label(&retention_label(settings.desired_retention));
                loading.set(false);
            }
            Err(err) => log_failure("reading the settings", &err.to_string()),
        }
    ));
    Context {
        state: state.clone(),
        changed: Rc::new(Cell::new(false)),
        loading,
        refresh,
        retention_busy: Rc::new(Cell::new(false)),
        errors: Rc::default(),
    }
}

/// Saves the two limits and the day start as they change.
fn connect_study_rows(rows: &Rows, ctx: &Context) {
    connect_limit(
        &rows.dialog,
        &rows.new_per_day,
        ctx,
        AppState::set_max_new_per_day,
    );
    connect_limit(
        &rows.dialog,
        &rows.reviews_per_day,
        ctx,
        AppState::set_max_reviews_per_day,
    );
    rows.day_start.connect_selected_notify(glib::clone!(
        #[weak(rename_to = dialog)]
        rows.dialog,
        #[strong]
        ctx,
        move |row| {
            if ctx.loading.get() {
                return;
            }
            let Some(hour) = hour_from_position(row.selected()) else {
                return;
            };
            let result = ctx.state.set_day_start_hour(hour);
            saved(&ctx, &dialog, "saving the day start", result);
        }
    ));
}

/// Opens the retention question when its row is activated.
fn connect_retention(window: &RappelWindow, rows: &Rows, ctx: &Context) {
    rows.retention_row.connect_activated(glib::clone!(
        #[weak]
        window,
        #[weak(rename_to = dialog)]
        rows.dialog,
        #[weak(rename_to = page)]
        rows.page,
        #[weak(rename_to = value)]
        rows.retention_value,
        #[strong]
        ctx,
        move |_| {
            // A key repeat must not open a second question.
            if ctx.retention_busy.replace(true) {
                return;
            }
            glib::spawn_future_local(glib::clone!(
                #[strong]
                ctx,
                async move {
                    change_retention(&window, &dialog, &page, &value, &ctx).await;
                    ctx.retention_busy.set(false);
                }
            ));
        }
    ));
}

/// Opens the backup folder when its button is activated.
fn connect_backups(window: &RappelWindow, rows: &Rows, ctx: &Context) {
    rows.open_backups.connect_activated(glib::clone!(
        #[weak]
        window,
        #[weak(rename_to = dialog)]
        rows.dialog,
        #[strong]
        ctx,
        move |_| {
            let ctx = ctx.clone();
            glib::spawn_future_local(async move {
                open_backup_folder(&window, &dialog, &ctx.state, &ctx.errors).await;
            });
        }
    ));
}

/// Shows `folder` (or "Off") in the Automatic Backups row, with its full
/// path as the row's tooltip, and Turn Off only while a folder is set.
fn show_auto_backup(row: &adw::ActionRow, off: &gtk::Button, folder: Option<&Path>) {
    row.set_subtitle(&auto_backup::folder_subtitle(folder));
    let full = folder.map(|f| f.display().to_string());
    row.set_tooltip_text(full.as_deref());
    // Tooltips are not reliably read out: the full path is also the row's
    // accessible description (`GtkAccessible`, `update_property`).
    // Through `GtkWidget`: the binding marks `AdwActionRow` as accessible
    // only with its `gtk_v4_10` feature, though every widget is one.
    let widget = row.upcast_ref::<gtk::Widget>();
    match &full {
        Some(path) => widget.update_property(&[gtk::accessible::Property::Description(path)]),
        None => widget.reset_property(gtk::AccessibleProperty::Description),
    }
    off.set_visible(folder.is_some());
}

/// Choose… asks for a folder; Turn Off turns automatic backups off. Both
/// save at once.
fn connect_auto_backup(window: &RappelWindow, rows: &Rows, ctx: &Context) {
    let row = Rc::new(AutoBackupRow {
        dialog: rows.dialog.downgrade(),
        row: rows.auto_backup_row.downgrade(),
        choose: rows.auto_backup_choose.downgrade(),
        off: rows.auto_backup_off.downgrade(),
    });
    let choosing = Rc::new(Cell::new(false));
    rows.auto_backup_choose.connect_clicked(glib::clone!(
        #[weak]
        window,
        #[strong]
        row,
        #[strong]
        ctx,
        move |_| {
            // A second click while the chooser is open does nothing.
            if choosing.replace(true) {
                return;
            }
            let (row, ctx, choosing) = (row.clone(), ctx.clone(), choosing.clone());
            glib::spawn_future_local(async move {
                choose_backup_folder(&window, &row, &ctx).await;
                choosing.set(false);
            });
        }
    ));
    rows.auto_backup_off.connect_clicked(glib::clone!(
        #[strong]
        ctx,
        move |_| row.save(&ctx, None)
    ));
}

/// The Automatic Backups row's widgets, held weakly so a handler never
/// keeps the dialog alive.
struct AutoBackupRow {
    dialog: glib::WeakRef<adw::PreferencesDialog>,
    row: glib::WeakRef<adw::ActionRow>,
    choose: glib::WeakRef<gtk::Button>,
    off: glib::WeakRef<gtk::Button>,
}

impl AutoBackupRow {
    /// Saves `folder` as the setting and shows it; a failed write says so
    /// and shows the setting as it is.
    fn save(&self, ctx: &Context, folder: Option<&Path>) {
        let (Some(dialog), Some(row), Some(choose), Some(off)) = (
            self.dialog.upgrade(),
            self.row.upgrade(),
            self.choose.upgrade(),
            self.off.upgrade(),
        ) else {
            return;
        };
        let data_dir = ctx.state.data_dir();
        if let Err(err) = auto_backup::set_folder(data_dir, folder) {
            ctx.errors.show_in_preferences(
                &dialog,
                "saving the automatic backup folder",
                // Translators: a toast: what is before ":" is its title, the rest is behind Details. {error} is why, in English.
                &with_error(&gettext("The setting was not saved: {error}"), &err),
            );
        }
        // Turn Off hidden under the focus hands the focus to Choose…, as
        // the next control of the row.
        let had_focus = off.has_focus();
        show_auto_backup(&row, &off, auto_backup::load(data_dir).folder.as_deref());
        if had_focus && !off.is_visible() {
            choose.grab_focus();
        }
    }
}

/// Asks for the automatic backup folder through `GtkFileDialog`, which
/// uses the desktop portal's file chooser when sandboxed (a snap, a
/// Flatpak), so the folder chosen is one the app may write to. The path
/// is kept with its links resolved (`canonicalize`): a link found in its
/// place later was put there since, and is refused (ADR 0080).
async fn choose_backup_folder(window: &RappelWindow, rows: &AutoBackupRow, ctx: &Context) {
    let Some(dialog) = rows.dialog.upgrade() else {
        return;
    };
    let chooser = gtk::FileDialog::builder()
        .title(gettext("Choose Backup Folder"))
        .modal(true)
        .build();
    if let Some(current) = auto_backup::load(ctx.state.data_dir()).folder {
        chooser.set_initial_folder(Some(&gio::File::for_path(current)));
    }
    let chosen = match chooser.select_folder_future(Some(window)).await {
        Ok(file) => file,
        // Closing the chooser is not an error (GtkFileDialog docs: the
        // callback gets GTK_DIALOG_ERROR_DISMISSED).
        Err(err)
            if err.matches(gtk::DialogError::Dismissed)
                || err.matches(gtk::DialogError::Cancelled) =>
        {
            return;
        }
        Err(err) => {
            ctx.errors.show_in_preferences(
                &dialog,
                "choosing the automatic backup folder",
                // Translators: a toast: what is before ":" is its title, the rest is behind Details. {error} is why, in English.
                &with_error(&gettext("The folder chooser failed: {error}"), &err),
            );
            return;
        }
    };
    // Links resolved now, or the choice is refused: a path that still ends
    // in a link would fail every backup as "replaced by a link".
    let Some(path) = chosen
        .path()
        .and_then(|path| std::fs::canonicalize(path).ok())
        .filter(|path| path.to_str().is_some())
    else {
        ctx.errors.show_in_preferences(
            &dialog,
            "choosing the automatic backup folder",
            &gettext("Choose a folder on this computer"),
        );
        return;
    };
    if rappel_core::auto_backup::inside_data_dir(&path, ctx.state.data_dir()) {
        ctx.errors.show_in_preferences(
            &dialog,
            "choosing the automatic backup folder",
            &auto_backup::inside_data_dir_text(),
        );
        return;
    }
    rows.save(ctx, Some(&path));
}

/// Saves a daily limit whenever its row's value changes.
fn connect_limit(
    dialog: &adw::PreferencesDialog,
    row: &adw::SpinRow,
    ctx: &Context,
    set: fn(&AppState, u32) -> rappel_core::Result<()>,
) {
    // `AdwSpinRow` applies typed text on Enter or when it loses the focus,
    // and then changes its adjustment (libadwaita `adw-spin-row.c`).
    row.adjustment().connect_value_changed(glib::clone!(
        #[weak]
        dialog,
        #[weak]
        row,
        #[strong]
        ctx,
        move |_| {
            if ctx.loading.get() {
                return;
            }
            let result = set(&ctx.state, limit_from_value(row.value()));
            saved(&ctx, &dialog, "saving a daily limit", result);
        }
    ));
}

/// After a write: remembers that something changed, or says it failed and
/// shows the stored values again.
fn saved(
    ctx: &Context,
    dialog: &adw::PreferencesDialog,
    what: &str,
    result: rappel_core::Result<()>,
) {
    match result {
        Ok(()) => ctx.changed.set(true),
        Err(err) => {
            ctx.errors.show_in_preferences(
                dialog,
                what,
                // Translators: a toast: what is before ":" is its title, the rest is behind Details. {error} is why, in English.
                &with_error(&gettext("The setting was not saved: {error}"), &err),
            );
            (ctx.refresh)();
        }
    }
}

/// Asks what retention to use, and if confirmed rebuilds the schedule on a
/// worker thread. The dialog cannot be closed meanwhile, so the pages
/// behind are read again only when the new schedule is in place.
async fn change_retention(
    window: &RappelWindow,
    dialog: &adw::PreferencesDialog,
    page: &adw::PreferencesPage,
    value_label: &gtk::Label,
    ctx: &Context,
) {
    let current = match ctx.state.settings() {
        Ok(settings) => percent_from_retention(settings.desired_retention),
        Err(err) => {
            ctx.errors.show_in_preferences(
                dialog,
                "reading the settings",
                // Translators: a toast: what is before ":" is its title, the rest is behind Details. {error} is why, in English.
                &with_error(&gettext("The settings could not be read: {error}"), &err),
            );
            return;
        }
    };
    let Ok((alert, spin)) = retention_layout() else {
        return;
    };
    spin.set_value(f64::from(current));
    // Nothing to confirm while the value is the one already in use.
    alert.set_response_enabled("change", false);
    spin.adjustment().connect_value_changed(glib::clone!(
        #[weak]
        alert,
        move |adjustment| {
            alert.set_response_enabled("change", percent_from_value(adjustment.value()) != current);
        }
    ));
    if alert.choose_future(Some(dialog)).await != "change" {
        return;
    }
    let retention = retention_from_percent(percent_from_value(spin.value()));

    value_label.set_label(&gettext("Recalculating…"));
    // Every row waits: the worker holds the write lock, and a save on the
    // main connection would block the window until it gives up.
    page.set_sensitive(false);
    dialog.set_can_close(false);
    let data_dir = ctx.state.data_dir().to_path_buf();
    let result = worker::run(window, move || {
        AppState::set_desired_retention_on_worker(&data_dir, retention)
    })
    .await;
    dialog.set_can_close(true);
    page.set_sensitive(true);
    match result {
        Ok(Ok(())) => ctx.changed.set(true),
        Ok(Err(err)) => {
            ctx.errors.show_in_preferences(
                dialog,
                "changing the desired retention",
                // Translators: a toast: what is before ":" is its title, the rest is behind Details. {error} is why, in English.
                &with_error(
                    &gettext("The desired retention was not changed: {error}"),
                    &err,
                ),
            );
        }
        Err(_) => {
            ctx.errors.show_in_preferences(
                dialog,
                "changing the desired retention",
                &gettext("Changing the desired retention stopped unexpectedly"),
            );
        }
    }
    // Either way, the label shows what is stored.
    (ctx.refresh)();
}

/// Opens the backup folder in the file manager, creating it first if no
/// backup was made yet. `GtkFileLauncher` hands the folder to the system's
/// default handler for it (through the desktop portal when sandboxed), so
/// the app itself does nothing network related.
async fn open_backup_folder(
    window: &RappelWindow,
    dialog: &adw::PreferencesDialog,
    state: &AppState,
    errors: &ErrorSlot,
) {
    let folder = state.backup_dir();
    if let Err(err) = ensure_data_dir(&folder) {
        errors.show_in_preferences(
            dialog,
            "creating the backup folder",
            // Translators: a toast: what is before ":" is its title, the rest is behind Details. {error} is why, in English.
            &with_error(
                &gettext("The backup folder could not be created: {error}"),
                &err,
            ),
        );
        return;
    }
    let launcher = gtk::FileLauncher::new(Some(&gio::File::for_path(&folder)));
    if let Err(err) = launcher.launch_future(Some(window)).await {
        errors.show_in_preferences(
            dialog,
            "opening the backup folder",
            // Translators: a toast: what is before ":" is its title, the rest is behind Details. {error} is why, in English.
            &with_error(
                &gettext("The backup folder could not be opened: {error}"),
                &err,
            ),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn percent_and_retention_convert_both_ways() {
        for percent in 80..=97_u8 {
            let retention = retention_from_percent(percent);
            assert_eq!(percent_from_retention(retention), percent);
            // The store accepts every one of them.
            assert_eq!(Settings::check_retention(retention).unwrap(), retention);
        }
        // The values the store knows as literals.
        assert_eq!(retention_from_percent(90), 0.90);
        assert_eq!(retention_from_percent(80), 0.80);
        assert_eq!(retention_from_percent(97), 0.97);
    }

    #[test]
    fn the_percent_of_a_spin_row_stays_in_range() {
        assert_eq!(percent_from_value(79.0), 80);
        assert_eq!(percent_from_value(80.4), 80);
        assert_eq!(percent_from_value(89.6), 90);
        assert_eq!(percent_from_value(97.4), 97);
        assert_eq!(percent_from_value(250.0), 97);
        assert_eq!(percent_from_value(f64::NAN), 80);
    }

    #[test]
    fn the_retention_label() {
        assert_eq!(retention_label(0.90), "90%");
        assert_eq!(retention_label(0.8), "80%");
        assert_eq!(retention_label(0.97), "97%");
    }

    #[test]
    fn limits_from_spin_rows() {
        assert_eq!(limit_from_value(0.0), 0);
        assert_eq!(limit_from_value(19.6), 20);
        assert_eq!(limit_from_value(9_999.0), MAX_DAILY_LIMIT);
        assert_eq!(limit_from_value(20_000.0), MAX_DAILY_LIMIT);
        assert_eq!(limit_from_value(-5.0), 0);
        assert_eq!(limit_from_value(f64::NAN), 0);
    }

    #[test]
    fn hours() {
        assert_eq!(hour_label(0), "00:00");
        assert_eq!(hour_label(4), "04:00");
        assert_eq!(hour_label(23), "23:00");
        assert_eq!(hour_from_position(0), Some(0));
        assert_eq!(hour_from_position(23), Some(23));
        assert_eq!(hour_from_position(24), None);
        // `GTK_INVALID_LIST_POSITION`: nothing selected.
        assert_eq!(hour_from_position(u32::MAX), None);
    }

    /// The layouts load with every object this file asks for, the rows show
    /// the settings without saving them, and a change the user makes is
    /// saved. (A wrong object id in a `.blp` file would only show when the
    /// dialog is opened.)
    #[test]
    fn the_rows_show_and_save_the_settings() {
        crate::testing::with_widgets(|| {
            let (_tmp, state) = crate::testing::temporary_state();

            let rows = Rows::load().expect("preferences_dialog.ui has every object");
            let ctx = context(&rows, &state);
            let shown = Settings {
                desired_retention: 0.85,
                max_new_per_day: 7,
                max_reviews_per_day: 300,
                day_start_hour: 23,
            };
            rows.fill(&ctx.loading, &shown);
            connect_study_rows(&rows, &ctx);
            assert_eq!(rows.new_per_day.value(), 7.0);
            assert_eq!(rows.reviews_per_day.value(), 300.0);
            assert_eq!(rows.day_start.selected(), 23);
            assert_eq!(rows.retention_value.label(), "85%");
            assert_eq!(
                rows.day_start.model().map(|m| m.n_items()),
                Some(24),
                "one entry per hour"
            );
            // Filling the rows is not a change by the user.
            assert!(!ctx.changed.get());
            assert_eq!(state.settings().unwrap(), Settings::default());

            rows.new_per_day.set_value(11.0);
            rows.reviews_per_day.set_value(50.0);
            rows.day_start.set_selected(6);
            let saved = state.settings().unwrap();
            assert_eq!(
                (
                    saved.max_new_per_day,
                    saved.max_reviews_per_day,
                    saved.day_start_hour
                ),
                (11, 50, 6)
            );
            assert_eq!(saved.desired_retention, 0.90, "not touched by these rows");
            assert!(ctx.changed.get());

            // `refresh` shows the stored values again, as after a failed write
            // that left a row showing what was not saved.
            ctx.loading.set(true);
            rows.new_per_day.set_value(3.0);
            ctx.loading.set(false);
            assert_eq!(state.settings().unwrap().max_new_per_day, 11);
            (ctx.refresh)();
            assert_eq!(rows.new_per_day.value(), 11.0);
            assert_eq!(rows.retention_value.label(), "90%");
        });
    }

    /// The Automatic Backups row shows "Off" until a folder is chosen,
    /// then the folder's name with Turn Off; both save the setting.
    #[test]
    fn the_automatic_backup_row_shows_and_saves_the_folder() {
        crate::testing::with_widgets(|| {
            let (tmp, state) = crate::testing::temporary_state();
            let rows = Rows::load().expect("preferences_dialog.ui has every object");
            let ctx = context(&rows, &state);
            show_auto_backup(&rows.auto_backup_row, &rows.auto_backup_off, None);
            assert_eq!(rows.auto_backup_row.subtitle().as_deref(), Some("Off"));
            assert!(!rows.auto_backup_off.is_visible());

            let row = AutoBackupRow {
                dialog: rows.dialog.downgrade(),
                row: rows.auto_backup_row.downgrade(),
                choose: rows.auto_backup_choose.downgrade(),
                off: rows.auto_backup_off.downgrade(),
            };
            let folder = tmp.path().join("My Backups");
            row.save(&ctx, Some(&folder));
            assert_eq!(
                auto_backup::load(state.data_dir()).folder.as_deref(),
                Some(folder.as_path())
            );
            assert_eq!(
                rows.auto_backup_row.subtitle().as_deref(),
                Some("My Backups")
            );
            assert_eq!(
                rows.auto_backup_row.tooltip_text().as_deref(),
                folder.to_str()
            );
            assert!(rows.auto_backup_off.is_visible());

            row.save(&ctx, None);
            assert_eq!(auto_backup::load(state.data_dir()).folder, None);
            assert_eq!(rows.auto_backup_row.subtitle().as_deref(), Some("Off"));
            assert!(!rows.auto_backup_off.is_visible());
        });
    }

    #[test]
    fn the_retention_question_loads_with_its_range() {
        crate::testing::with_widgets(|| {
            let (alert, spin) = retention_layout().expect("retention_dialog.ui has every object");
            let adjustment = spin.adjustment();
            assert_eq!(
                (adjustment.lower(), adjustment.upper()),
                (
                    f64::from(percent_from_retention(*DESIRED_RETENTION_RANGE.start())),
                    f64::from(percent_from_retention(*DESIRED_RETENTION_RANGE.end()))
                )
            );
            assert_eq!(alert.default_response().as_deref(), Some("change"));
            // The limit rows' range is the store's.
            let b = dialog::builder(PREFERENCES_FILE);
            for id in ["new_per_day", "reviews_per_day"] {
                let row: adw::SpinRow = object(&b, id).unwrap();
                let adjustment = row.adjustment();
                assert_eq!(
                    (adjustment.lower(), adjustment.upper()),
                    (0.0, f64::from(MAX_DAILY_LIMIT)),
                    "{id}"
                );
            }
            assert_eq!(alert.close_response(), "cancel");
        });
    }
}
