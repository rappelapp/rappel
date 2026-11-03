//! Export and import from the primary menu (SPEC §4.1, ADR 0018).
//!
//! The file choosers and dialogs run here on the main thread; the work runs
//! on a worker thread with its own database connection
//! ([`AppState::export_on_worker`] and the import functions), so the window
//! never freezes. The texts are pure functions in [`text`]; [`names`]
//! looks up the user's own names that conflicts refer to.

mod anki;
mod anki_text;
mod names;
mod progress;
mod text;

use crate::i18n::{fill, gettext};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use adw::prelude::*;
use gtk::{gio, glib};
use rappel_core::Error;
use rappel_core::interchange::{
    ANKI_EXTENSIONS, FILE_EXTENSION, FileKind, IMPORT_EXTENSIONS, ImportPlan, ImportSummary,
    media_too_large_for_import,
};

use crate::state::AppState;
use crate::ui::toast::{info_toast, persistent_toast, show_error};
use crate::ui::{dialog, worker};
use crate::window::RappelWindow;
use anki_text::cancelled_text;
use names::Names;
use progress::Progress;
use text::{
    OverLimit, check_failed_text, checking_text, conflicts_text, export_file_name, exported_text,
    exported_without_media_text, import_failed_text, imported_text, plan_view, refused_text,
    too_large_without_media_text, without_media_body,
};

/// Asks where to save, then exports there on a worker thread. If the media
/// would make the file larger than an import reads, asks first (before
/// anything is chosen or written) whether to export without them; if the
/// file comes out too large all the same, asks then (ADR 0071).
pub async fn export(window: &RappelWindow, state: &AppState) {
    let include_media = match media_choice(window, state).await {
        MediaChoice::With => true,
        MediaChoice::Without => false,
        MediaChoice::Cancel => return,
    };
    let (year, month, day) = glib::DateTime::now_local().map_or((0, 0, 0), |now| now.ymd());
    let dialog = file_dialog(&gettext("Export Data"), &[FILE_EXTENSION]);
    dialog.set_initial_name(Some(&export_file_name(year, month, day)));
    let Some(path) = chosen_path(window, dialog.save_future(Some(window)).await) else {
        return;
    };

    let data_dir = state.data_dir().to_path_buf();
    let mut include_media = include_media;
    loop {
        let (dir, target) = (data_dir.clone(), path.clone());
        let result = worker::run(window, move || {
            AppState::export_on_worker(&dir, &target, include_media)
        })
        .await;
        // The bound before writing leaves out the text: the file came out too
        // large. Not a failure to report: the question, then once more.
        if let Ok(Err(err)) = &result
            && let Some(why) = offers_export_without_media(err, include_media, has_media(state))
        {
            match ask(window, why).await {
                MediaChoice::Without => {
                    include_media = false;
                    continue;
                }
                MediaChoice::With | MediaChoice::Cancel => return,
            }
        }
        report(window, &path, include_media, result);
        return;
    }
}

/// Says how the export went.
fn report(
    window: &RappelWindow,
    path: &std::path::Path,
    include_media: bool,
    result: Result<rappel_core::Result<rappel_core::interchange::ExportSummary>, worker::Panicked>,
) {
    match result {
        Ok(Ok(summary)) => {
            // Lost media files are worth reading, so that toast stays; so
            // does the one that says the file has none on purpose.
            let toast = if include_media {
                let text = exported_text(path, summary.missing_media);
                if summary.missing_media > 0 {
                    persistent_toast(&text)
                } else {
                    info_toast(&text)
                }
            } else {
                persistent_toast(&exported_without_media_text(path, summary.media_left_out))
            };
            window.add_toast(toast);
        }
        // Without media, or with none to leave out: nothing more to leave
        // out, so not core's message, which is for the logs. (With media
        // to leave out, `export` asked instead and never gets here.)
        Ok(Err(Error::ExportTooLarge { bytes, .. })) => {
            show_error(window, "an export", &too_large_without_media_text(bytes));
        }
        Ok(Err(err)) => show_error(
            window,
            "an export",
            // Translators: a toast: what is before ":" is its title, the
            // rest is behind Details. {error} is why, in English.
            &fill(
                &gettext("The export failed: {error}"),
                &[("error", &err.to_string())],
            ),
        ),
        Err(_) => show_error(
            window,
            "an export",
            &gettext("The export stopped unexpectedly"),
        ),
    }
}

/// Why an export that failed with `err` is offered again without media, if
/// it is: it came out larger than an import reads, it included the media,
/// and there are media to leave out (`has_media`); else writing it again
/// would only fail the same way, and the failure is reported.
fn offers_export_without_media(
    err: &Error,
    include_media: bool,
    has_media: bool,
) -> Option<OverLimit> {
    match err {
        Error::ExportTooLarge { bytes, .. } if include_media && has_media => {
            Some(OverLimit::Written { bytes: *bytes })
        }
        _ => None,
    }
}

/// Whether the database lists any media file. If they cannot be counted,
/// taken as yes: the question is then asked, and Cancel stays possible.
fn has_media(state: &AppState) -> bool {
    state.media_totals().map_or(true, |totals| totals.files > 0)
}

/// What the export does about the media.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MediaChoice {
    /// Export them, as always.
    With,
    /// Leave them out.
    Without,
    /// Do not export.
    Cancel,
}

/// What an answer of "Export Without Media?" means: only its button.
fn media_choice_of(response: &str) -> MediaChoice {
    if response == "without_media" {
        MediaChoice::Without
    } else {
        MediaChoice::Cancel
    }
}

/// Asks, before anything is written, if the media alone make the file
/// larger than an import reads (2 GiB, ADR 0071). Cancel is the default
/// response and the way out, so Enter and Escape never export. If the
/// media cannot be added up, the export goes on as always: it checks the
/// file it wrote.
async fn media_choice(window: &RappelWindow, state: &AppState) -> MediaChoice {
    let totals = match state.media_totals() {
        Ok(totals) => totals,
        Err(err) => {
            tracing::warn!("cannot add up the media before an export: {err}");
            return MediaChoice::With;
        }
    };
    match media_too_large_for_import(totals) {
        Some(over) => ask(window, over.into()).await,
        None => MediaChoice::With,
    }
}

/// "Export Without Media?" over `window`, saying `why`. If the dialog
/// cannot be built, says so in a toast and does not export.
async fn ask(window: &RappelWindow, why: OverLimit) -> MediaChoice {
    let Ok(dialog) = dialog::EXPORT_WITHOUT_MEDIA.load() else {
        // `Alert::load` logged which object is missing.
        show_error(
            window,
            "an export",
            &gettext("The export could not start: its question could not be shown"),
        );
        return MediaChoice::Cancel;
    };
    dialog.set_body(&without_media_body(why));
    media_choice_of(&dialog.choose_future(Some(window)).await)
}

/// Asks for a file, checks it on a worker thread, shows what it would do,
/// and imports it on a worker thread if the user confirms. An Anki package
/// (by its name, else its content) goes to the Anki import ([`anki`]).
pub async fn import(window: &RappelWindow, state: &AppState) {
    let dialog = import_dialog();
    let Some(path) = chosen_path(window, dialog.open_future(Some(window)).await) else {
        return;
    };

    let data_dir = state.data_dir().to_path_buf();
    let sniffed = path.clone();
    let kind = worker::run(window, move || AppState::file_kind_on_worker(&sniffed)).await;
    if let Ok(Some(FileKind::Anki)) = kind {
        anki::import(window, data_dir, path).await;
        return;
    }
    let worker_dir = data_dir.clone();
    // Reading a large file takes a while: a toast with Cancel (and Escape)
    // stays until the plan is made, as for an Anki package (ADR 0071).
    let cancel = Arc::new(AtomicBool::new(false));
    let progress = Progress::start(window, &checking_text(), &cancel);
    let worker_cancel = Arc::clone(&cancel);
    let result = worker::run(window, move || {
        AppState::plan_import_on_worker(&worker_dir, &path, &worker_cancel)
    })
    .await;
    progress.finish();
    let plan = match result {
        Ok(Ok(plan)) => plan,
        Ok(Err(Error::ImportCancelled)) => {
            window.add_toast(info_toast(&cancelled_text()));
            return;
        }
        Ok(Err(Error::ImportRefused(problem))) => {
            show_error(window, "an import check", &refused_text(&problem));
            return;
        }
        Ok(Err(err)) => {
            show_error(window, "an import check", &check_failed_text(&err));
            return;
        }
        Err(_) => {
            show_error(
                window,
                "an import check",
                &gettext("Checking the file stopped unexpectedly"),
            );
            return;
        }
    };

    if !confirm(window, state, plan.summary()).await {
        return;
    }
    apply(window, data_dir, plan).await;
}

/// The import question and its conflicts label, from
/// `data/import_dialog.blp`; `Err` with the first missing object's id,
/// after logging every missing one.
pub(crate) fn confirm_layout() -> Result<(adw::AlertDialog, gtk::Label), &'static str> {
    const FILE: &str = "import_dialog.ui";
    let builder = dialog::builder(FILE);
    let (dialog, conflicts_label) = (
        dialog::object(&builder, FILE, "import_dialog"),
        dialog::object(&builder, FILE, "conflicts_label"),
    );
    Ok((dialog?, conflicts_label?))
}

/// Shows the plan; whether the user chose Import.
async fn confirm(window: &RappelWindow, state: &AppState, summary: &ImportSummary) -> bool {
    let Ok((dialog, conflicts_label)) = confirm_layout() else {
        return false;
    };
    let view = plan_view(summary);
    dialog.set_heading(Some(&view.heading));
    dialog.set_body(&view.body);
    if !view.can_import {
        dialog.remove_response("import");
        dialog.set_response_label("cancel", &gettext("_Close"));
        dialog.set_default_response(Some("cancel"));
    }
    if !summary.conflicts.is_empty() {
        let lookup = Names::read(state, &summary.conflicts);
        conflicts_label.set_label(&conflicts_text(
            &summary.conflicts,
            |id| lookup.deck(id),
            |id| lookup.card(id),
        ));
        conflicts_label.set_visible(true);
    }
    let response = dialog.choose_future(Some(window)).await;
    view.can_import && response == "import"
}

/// Applies the plan on a worker thread, then says what was added and
/// refreshes every page that shows decks or cards.
async fn apply(window: &RappelWindow, data_dir: PathBuf, plan: ImportPlan) {
    let result = worker::run(window, move || {
        AppState::apply_import_on_worker(&data_dir, &plan)
    })
    .await;
    match result {
        Ok(Ok(report)) => {
            window.add_toast(info_toast(&imported_text(&report)));
            window.data_changed();
        }
        Ok(Err(err)) => show_error(window, "an import", &import_failed_text(&err)),
        Err(_) => show_error(
            window,
            "an import",
            &gettext("The import stopped unexpectedly"),
        ),
    }
}

/// A file filter named `name` for the name `extensions`.
fn suffix_filter(name: &str, extensions: &[&str]) -> gtk::FileFilter {
    let filter = gtk::FileFilter::new();
    filter.set_name(Some(name));
    for extension in extensions {
        filter.add_suffix(extension);
    }
    filter
}

/// A modal file chooser for `.rappel` files, for saving an export: only
/// `.rappel`, so an old `.cards` export is not overwritten by accident.
fn file_dialog(title: &str, extensions: &[&str]) -> gtk::FileDialog {
    let filter = suffix_filter(&gettext("Rappel Exports"), extensions);
    let filters = gio::ListStore::new::<gtk::FileFilter>();
    filters.append(&filter);
    gtk::FileDialog::builder()
        .title(title)
        .modal(true)
        .filters(&filters)
        .default_filter(&filter)
        .build()
}

/// The modal file chooser of Import…: Rappel exports (`.rappel`, and
/// `.cards`, the name exports had before the rename, ADR 0020) and Anki
/// packages (`.apkg`, `.colpkg`, ADR 0061), together by default, or each
/// kind alone.
fn import_dialog() -> gtk::FileDialog {
    let all: Vec<&str> = IMPORT_EXTENSIONS
        .iter()
        .chain(&ANKI_EXTENSIONS)
        .copied()
        .collect();
    let both = suffix_filter(&gettext("Rappel Exports and Anki Packages"), &all);
    let filters = gio::ListStore::new::<gtk::FileFilter>();
    filters.append(&both);
    filters.append(&suffix_filter(
        &gettext("Rappel Exports"),
        &IMPORT_EXTENSIONS,
    ));
    filters.append(&suffix_filter(&gettext("Anki Packages"), &ANKI_EXTENSIONS));
    gtk::FileDialog::builder()
        .title(gettext("Import Data"))
        .modal(true)
        .filters(&filters)
        .default_filter(&both)
        .build()
}

/// The local path chosen, or `None` if the user cancelled or the chooser
/// failed (then after a toast).
fn chosen_path(window: &RappelWindow, chosen: Result<gio::File, glib::Error>) -> Option<PathBuf> {
    match chosen {
        Ok(file) => {
            let path = file.path();
            if path.is_none() {
                show_error(
                    window,
                    "a file choice",
                    &gettext("Choose a file on this computer"),
                );
            }
            path
        }
        // Closing the chooser is not an error (GtkFileDialog docs: the
        // callback gets GTK_DIALOG_ERROR_DISMISSED).
        Err(err)
            if err.matches(gtk::DialogError::Dismissed)
                || err.matches(gtk::DialogError::Cancelled) =>
        {
            None
        }
        Err(err) => {
            show_error(
                window,
                "a file choice",
                // Translators: a toast: what is before ":" is its title, the
                // rest is behind Details. {error} is why, in English.
                &fill(
                    &gettext("The file chooser failed: {error}"),
                    &[("error", &err.to_string())],
                ),
            );
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Only the Export Without Media button exports without them: Cancel,
    /// Escape (the close response) and anything else do not export.
    #[test]
    fn only_its_button_exports_without_media() {
        assert_eq!(media_choice_of("without_media"), MediaChoice::Without);
        assert_eq!(media_choice_of("cancel"), MediaChoice::Cancel);
        assert_eq!(media_choice_of("close"), MediaChoice::Cancel);
        assert_eq!(media_choice_of(""), MediaChoice::Cancel);
    }

    /// A file that came out too large is offered again without media, only
    /// when it had media and only for that error.
    #[test]
    fn only_a_too_large_export_with_media_is_offered_again_without() {
        let large = Error::ExportTooLarge { bytes: 5, limit: 4 };
        assert_eq!(
            offers_export_without_media(&large, true, true),
            Some(OverLimit::Written { bytes: 5 })
        );
        // Already without media: nothing more to leave out.
        assert_eq!(offers_export_without_media(&large, false, true), None);
        // No media at all: leaving them out would change nothing.
        assert_eq!(offers_export_without_media(&large, true, false), None);
        assert_eq!(
            offers_export_without_media(&Error::ImportCancelled, true, true),
            None
        );
    }

    #[test]
    fn the_question_defaults_to_cancel_and_its_button_names_what_it_does() {
        crate::testing::with_widgets(|| {
            let dialog = dialog::EXPORT_WITHOUT_MEDIA.load().unwrap();
            assert_eq!(dialog.default_response().as_deref(), Some("cancel"));
            assert_eq!(dialog.close_response(), "cancel");
            assert_eq!(
                dialog.response_label("without_media"),
                "_Export Without Media"
            );
            assert_eq!(dialog.response_label("cancel"), "_Cancel");
            assert_eq!(
                dialog.response_appearance("without_media"),
                adw::ResponseAppearance::Default
            );
        });
    }
}
