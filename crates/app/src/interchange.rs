//! Export and import from the primary menu (SPEC §4.1, ADR 0018).
//!
//! The file choosers and dialogs run here on the main thread; the work runs
//! on a worker thread with its own database connection
//! ([`AppState::export_on_worker`] and the import functions), so the window
//! never freezes. The texts are pure functions, unit-tested below.

use std::path::{Path, PathBuf};

use adw::prelude::*;
use cards_core::Error;
use cards_core::interchange::{Conflict, FILE_EXTENSION, ImportPlan, ImportReport, ImportSummary};
use gtk::{gio, glib};

use crate::state::AppState;
use crate::ui::dialog;
use crate::ui::text::{count, decks_cards_reviews};
use crate::ui::toast::{log_failure, persistent_toast};
use crate::widgets::front_preview;
use crate::window::CardsWindow;

/// How many conflicts the "Cannot Import" dialog lists by name.
const SHOWN_CONFLICTS: usize = 5;

/// Asks where to save, then exports there on a worker thread.
pub async fn export(window: &CardsWindow, state: &AppState) {
    let (year, month, day) = glib::DateTime::now_local().map_or((0, 0, 0), |now| now.ymd());
    let dialog = file_dialog("Export Data");
    dialog.set_initial_name(Some(&export_file_name(year, month, day)));
    let Some(path) = chosen_path(window, dialog.save_future(Some(window)).await) else {
        return;
    };

    let data_dir = state.data_dir().to_path_buf();
    let worker_path = path.clone();
    let result = {
        let _hold = hold(window);
        gio::spawn_blocking(move || AppState::export_on_worker(&data_dir, &worker_path)).await
    };
    match result {
        Ok(Ok(_)) => window.add_toast(
            adw::Toast::builder()
                .title(exported_text(&path))
                .use_markup(false)
                .build(),
        ),
        Ok(Err(err)) => fail(window, "an export", &format!("The export failed: {err}")),
        Err(_) => fail(window, "an export", "The export stopped unexpectedly"),
    }
}

/// Asks for a file, checks it on a worker thread, shows what it would do,
/// and imports it on a worker thread if the user confirms.
pub async fn import(window: &CardsWindow, state: &AppState) {
    let dialog = file_dialog("Import Data");
    let Some(path) = chosen_path(window, dialog.open_future(Some(window)).await) else {
        return;
    };

    let data_dir = state.data_dir().to_path_buf();
    let worker_dir = data_dir.clone();
    let result = {
        let _hold = hold(window);
        gio::spawn_blocking(move || AppState::plan_import_on_worker(&worker_dir, &path)).await
    };
    let plan = match result {
        Ok(Ok(plan)) => plan,
        Ok(Err(Error::ImportRefused(problem))) => {
            fail(
                window,
                "an import check",
                &format!("This file cannot be imported: {problem}"),
            );
            return;
        }
        Ok(Err(err)) => {
            fail(
                window,
                "an import check",
                &format!("The file could not be checked: {err}"),
            );
            return;
        }
        Err(_) => {
            fail(
                window,
                "an import check",
                "Checking the file stopped unexpectedly",
            );
            return;
        }
    };

    if !confirm(window, state, plan.summary()).await {
        return;
    }
    apply(window, data_dir, plan).await;
}

/// Shows the plan; whether the user chose Import.
async fn confirm(window: &CardsWindow, state: &AppState, summary: &ImportSummary) -> bool {
    const FILE: &str = "import_dialog.ui";
    let builder = dialog::builder(FILE);
    let (Some(dialog), Some(conflicts_label)) = (
        dialog::object::<adw::AlertDialog>(&builder, FILE, "import_dialog"),
        dialog::object::<gtk::Label>(&builder, FILE, "conflicts_label"),
    ) else {
        return false;
    };
    let view = plan_view(summary);
    dialog.set_heading(Some(view.heading));
    dialog.set_body(&view.body);
    if !view.can_import {
        dialog.remove_response("import");
        dialog.set_response_label("cancel", "_Close");
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
async fn apply(window: &CardsWindow, data_dir: PathBuf, plan: ImportPlan) {
    let result = {
        let _hold = hold(window);
        gio::spawn_blocking(move || AppState::apply_import_on_worker(&data_dir, &plan)).await
    };
    match result {
        Ok(Ok(report)) => {
            window.add_toast(
                adw::Toast::builder()
                    .title(imported_text(&report))
                    .use_markup(false)
                    .build(),
            );
            window.data_changed();
        }
        Ok(Err(Error::ImportPlanStale)) => fail(
            window,
            "an import",
            "Your data changed while the file was being checked. Nothing was imported; import it again.",
        ),
        Ok(Err(err)) => fail(
            window,
            "an import",
            &format!("The import failed, and nothing was imported: {err}"),
        ),
        Err(_) => fail(window, "an import", "The import stopped unexpectedly"),
    }
}

/// Keeps the application running while a worker runs, so closing the
/// window lets it finish instead of cutting it off, as a restore does. Only
/// around the worker: a chooser or dialog left open must not keep a
/// windowless process alive.
fn hold(window: &CardsWindow) -> Option<gio::ApplicationHoldGuard> {
    window.application().map(|app| app.hold())
}

/// A modal file chooser for `.cards` files.
fn file_dialog(title: &str) -> gtk::FileDialog {
    let filter = gtk::FileFilter::new();
    filter.set_name(Some("Cards Exports"));
    filter.add_suffix(FILE_EXTENSION);
    let filters = gio::ListStore::new::<gtk::FileFilter>();
    filters.append(&filter);
    gtk::FileDialog::builder()
        .title(title)
        .modal(true)
        .filters(&filters)
        .default_filter(&filter)
        .build()
}

/// The local path chosen, or `None` if the user cancelled or the chooser
/// failed (then after a toast).
fn chosen_path(window: &CardsWindow, chosen: Result<gio::File, glib::Error>) -> Option<PathBuf> {
    match chosen {
        Ok(file) => {
            let path = file.path();
            if path.is_none() {
                fail(window, "a file choice", "Choose a file on this computer");
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
            fail(
                window,
                "a file choice",
                &format!("The file chooser failed: {err}"),
            );
            None
        }
    }
}

/// Logs and shows `message` in a toast that stays until dismissed.
fn fail(window: &CardsWindow, what: &str, message: &str) {
    log_failure(what, message);
    window.add_toast(persistent_toast(message));
}

/// The name of the user's own deck or card that a conflict refers to.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Lookup {
    /// The deck's name, or the card's front on one line.
    Found(String),
    /// Not among the live rows: deleted.
    Deleted,
    /// The database could not be read; say nothing about it.
    Unknown,
}

/// The names of the user's own decks and cards that conflicts refer to.
/// Only the database is read, never the file, and only for the conflicts
/// that are shown.
struct Names {
    /// `None` if the decks could not be read.
    decks: Option<Vec<(String, String)>>,
    cards: Vec<(String, Lookup)>,
}

impl Names {
    fn read(state: &AppState, conflicts: &[Conflict]) -> Self {
        let decks = match state.list_decks() {
            Ok(decks) => Some(
                decks
                    .into_iter()
                    .map(|s| (s.deck.id, s.deck.name))
                    .collect(),
            ),
            Err(err) => {
                log_failure("reading deck names for import conflicts", &err.to_string());
                None
            }
        };
        let cards = conflicts
            .iter()
            .take(SHOWN_CONFLICTS)
            .filter_map(|c| match c {
                Conflict::CardDiffers(id) => Some((id.clone(), card_lookup(state, id))),
                _ => None,
            })
            .collect();
        Self { decks, cards }
    }

    fn deck(&self, id: &str) -> Lookup {
        let Some(decks) = &self.decks else {
            return Lookup::Unknown;
        };
        decks
            .iter()
            .find(|(i, _)| i == id)
            .map_or(Lookup::Deleted, |(_, name)| Lookup::Found(name.clone()))
    }

    fn card(&self, id: &str) -> Lookup {
        self.cards
            .iter()
            .find(|(i, _)| i == id)
            .map_or(Lookup::Unknown, |(_, lookup)| lookup.clone())
    }
}

fn card_lookup(state: &AppState, id: &str) -> Lookup {
    match state.card(id) {
        Ok(card) => Lookup::Found(short_front(&card.front_md)),
        // `card` returns only live cards.
        Err(Error::CardNotFound(_)) => Lookup::Deleted,
        Err(err) => {
            log_failure("reading a card for import conflicts", &err.to_string());
            Lookup::Unknown
        }
    }
}

/// How many characters of a card front a conflict line shows.
const FRONT_CHARS: usize = 60;

/// The front on one line (as the card list shows it), cut to
/// [`FRONT_CHARS`] with "…" when anything was left out.
fn short_front(front: &str) -> String {
    let preview = front_preview(front);
    let words: Vec<&str> = front.split_whitespace().collect();
    let whole: usize =
        words.iter().map(|w| w.chars().count()).sum::<usize>() + words.len().saturating_sub(1);
    let shown: String = preview.chars().take(FRONT_CHARS).collect();
    if whole > shown.chars().count() {
        format!("{}…", shown.trim_end())
    } else {
        shown
    }
}

/// `cards-YYYY-MM-DD.cards`, the export's suggested name.
fn export_file_name(year: i32, month: i32, day: i32) -> String {
    if year <= 0 {
        // No local date: GLib could not read the time zone.
        return format!("cards.{FILE_EXTENSION}");
    }
    format!("cards-{year:04}-{month:02}-{day:02}.{FILE_EXTENSION}")
}

fn exported_text(path: &Path) -> String {
    let name = path.file_name().map_or_else(
        || path.display().to_string(),
        |n| n.to_string_lossy().into_owned(),
    );
    format!("Exported to “{name}”")
}

fn imported_text(report: &ImportReport) -> String {
    decks_cards_reviews(report.decks, report.cards, report.review_events).map_or_else(
        || "Nothing was imported: everything was already here".to_owned(),
        |added| format!("Imported {added}"),
    )
}

/// What the import dialog says.
#[derive(Debug, PartialEq, Eq)]
struct PlanView {
    heading: &'static str,
    body: String,
    /// Whether Import is offered.
    can_import: bool,
}

fn plan_view(summary: &ImportSummary) -> PlanView {
    let conflicts = summary.conflicts.len();
    if conflicts > 0 {
        return PlanView {
            heading: "Cannot Import",
            body: format!(
                "This file disagrees with your data in {}. Nothing is imported, \
                 and your data stays as it is.",
                count(conflicts, "place", "places")
            ),
            can_import: false,
        };
    }
    let new = decks_cards_reviews(
        summary.decks.new,
        summary.cards.new,
        summary.review_events.new,
    );
    let Some(new) = new else {
        return PlanView {
            heading: "Nothing to Import",
            body: "Everything in this file is already here.".to_owned(),
            can_import: false,
        };
    };
    let mut sentences = vec![format!("This adds {new}.")];
    let (decks, cards, reviews) = (
        summary.decks.present,
        summary.cards.present,
        summary.review_events.present,
    );
    if let Some(present) = decks_cards_reviews(decks, cards, reviews) {
        let verb = if decks + cards + reviews == 1 {
            "is"
        } else {
            "are"
        };
        sentences.push(format!(
            "{present} {verb} already here and will be skipped."
        ));
    }
    sentences.push("Your data is backed up first.".to_owned());
    let body = sentences.join(" ");
    PlanView {
        heading: "Import Data?",
        body,
        can_import: true,
    }
}

/// The first [`SHOWN_CONFLICTS`] conflicts, one per line, then how many
/// more. `deck_name` and `card_front` look up the user's own deck names
/// and card fronts.
fn conflicts_text(
    conflicts: &[Conflict],
    deck_name: impl Fn(&str) -> Lookup,
    card_front: impl Fn(&str) -> Lookup,
) -> String {
    let mut lines: Vec<String> = conflicts
        .iter()
        .take(SHOWN_CONFLICTS)
        .map(|conflict| {
            let line = match conflict {
                Conflict::DeckDiffers(id) => match deck_name(id) {
                    Lookup::Found(name) => format!("The deck “{name}” is different in the file"),
                    Lookup::Deleted => "A deleted deck is different in the file".to_owned(),
                    Lookup::Unknown => "A deck is different in the file".to_owned(),
                },
                Conflict::CardDiffers(id) => match card_front(id) {
                    Lookup::Found(front) => {
                        format!("The card “{front}” is different in the file")
                    }
                    Lookup::Deleted => "A deleted card is different in the file".to_owned(),
                    Lookup::Unknown => "A card is different in the file".to_owned(),
                },
                Conflict::EventDiffers(_) => {
                    "A review is recorded differently in the file".to_owned()
                }
                Conflict::DeckNameTaken { existing_id, .. } => match deck_name(existing_id) {
                    Lookup::Found(name) => format!("The file has another deck named “{name}”"),
                    Lookup::Deleted | Lookup::Unknown => {
                        "The file has another deck with the name of one of yours".to_owned()
                    }
                },
            };
            format!("• {line}")
        })
        .collect();
    let more = conflicts.len().saturating_sub(SHOWN_CONFLICTS);
    if more > 0 {
        lines.push(format!("and {more} more"));
    }
    lines.join("\n")
}

#[cfg(test)]
mod tests {
    use cards_core::interchange::TableCounts;

    use super::*;

    const DECK: &str = "01920000-0000-7000-8000-000000000001";
    const CARD: &str = "01920000-0000-7000-8000-000000000002";
    const EVENT: &str = "01920000-0000-7000-8000-000000000003";
    const OTHER: &str = "01920000-0000-7000-8000-000000000004";

    fn counts(new: usize, present: usize) -> TableCounts {
        TableCounts { new, present }
    }

    fn summary(decks: TableCounts, cards: TableCounts, reviews: TableCounts) -> ImportSummary {
        ImportSummary {
            decks,
            cards,
            review_events: reviews,
            conflicts: Vec::new(),
        }
    }

    #[test]
    fn the_export_is_named_by_the_local_date() {
        assert_eq!(export_file_name(2026, 10, 16), "cards-2026-10-16.cards");
        assert_eq!(export_file_name(2027, 1, 5), "cards-2027-01-05.cards");
        assert_eq!(export_file_name(0, 0, 0), "cards.cards");
    }

    #[test]
    fn the_export_toast_names_only_the_file() {
        assert_eq!(
            exported_text(Path::new("/home/me/Documents/cards-2026-10-16.cards")),
            "Exported to “cards-2026-10-16.cards”"
        );
    }

    #[test]
    fn new_rows_offer_import_and_name_what_is_already_here() {
        let view = plan_view(&summary(counts(12, 0), counts(840, 0), counts(5_210, 0)));
        assert_eq!(
            view,
            PlanView {
                heading: "Import Data?",
                body: "This adds 12 decks, 840 cards and 5,210 reviews. \
                       Your data is backed up first."
                    .to_owned(),
                can_import: true,
            }
        );
        let view = plan_view(&summary(counts(0, 1), counts(1, 0), counts(0, 0)));
        assert_eq!(
            view.body,
            "This adds 1 card. 1 deck is already here and will be skipped. \
             Your data is backed up first."
        );
        let view = plan_view(&summary(counts(1, 2), counts(0, 3), counts(0, 0)));
        assert_eq!(
            view.body,
            "This adds 1 deck. 2 decks and 3 cards are already here and will be skipped. \
             Your data is backed up first."
        );
    }

    #[test]
    fn nothing_new_offers_no_import() {
        let view = plan_view(&summary(counts(0, 2), counts(0, 5), counts(0, 9)));
        assert_eq!(view.heading, "Nothing to Import");
        assert!(!view.can_import);
        // An empty file too.
        assert!(!plan_view(&ImportSummary::default()).can_import);
    }

    #[test]
    fn conflicts_offer_no_import_whatever_is_new() {
        let mut s = summary(counts(3, 0), counts(9, 0), counts(0, 0));
        s.conflicts = vec![Conflict::DeckDiffers(DECK.to_owned())];
        let view = plan_view(&s);
        assert_eq!(view.heading, "Cannot Import");
        assert!(!view.can_import);
        assert_eq!(
            view.body,
            "This file disagrees with your data in 1 place. Nothing is imported, \
             and your data stays as it is."
        );
    }

    fn names(id: &str) -> Lookup {
        match id {
            DECK => Lookup::Found("Maths".to_owned()),
            OTHER => Lookup::Found("Physics".to_owned()),
            _ => Lookup::Deleted,
        }
    }

    fn fronts(id: &str) -> Lookup {
        if id == CARD {
            Lookup::Found("2 + 2 ?".to_owned())
        } else {
            Lookup::Deleted
        }
    }

    #[test]
    fn each_conflict_names_the_users_own_deck_or_card() {
        let conflicts = [
            Conflict::DeckDiffers(DECK.to_owned()),
            Conflict::DeckDiffers(EVENT.to_owned()),
            Conflict::CardDiffers(CARD.to_owned()),
            Conflict::CardDiffers(EVENT.to_owned()),
            Conflict::EventDiffers(EVENT.to_owned()),
        ];
        assert_eq!(
            conflicts_text(&conflicts, names, fronts),
            "• The deck “Maths” is different in the file\n\
             • A deleted deck is different in the file\n\
             • The card “2 + 2 ?” is different in the file\n\
             • A deleted card is different in the file\n\
             • A review is recorded differently in the file"
        );
        let taken = [Conflict::DeckNameTaken {
            deck_id: DECK.to_owned(),
            existing_id: OTHER.to_owned(),
        }];
        assert_eq!(
            conflicts_text(&taken, names, fronts),
            "• The file has another deck named “Physics”"
        );
    }

    #[test]
    fn unreadable_names_are_not_called_deleted() {
        let conflicts = [
            Conflict::DeckDiffers(DECK.to_owned()),
            Conflict::CardDiffers(CARD.to_owned()),
        ];
        assert_eq!(
            conflicts_text(&conflicts, |_| Lookup::Unknown, |_| Lookup::Unknown),
            "• A deck is different in the file\n\
             • A card is different in the file"
        );
    }

    #[test]
    fn a_cut_front_ends_with_an_ellipsis() {
        assert_eq!(short_front("2 +\n 2 ?"), "2 + 2 ?");
        let long = "word ".repeat(200);
        let short = short_front(&long);
        assert!(short.ends_with('…'));
        assert_eq!(short.chars().count(), FRONT_CHARS);
        assert!(!short.contains(" …"));
    }

    #[test]
    fn only_the_first_conflicts_are_listed_then_a_count() {
        let conflicts: Vec<Conflict> = (0..8)
            .map(|_| Conflict::EventDiffers(EVENT.to_owned()))
            .collect();
        let text = conflicts_text(&conflicts, names, fronts);
        assert_eq!(text.lines().count(), SHOWN_CONFLICTS + 1);
        assert_eq!(text.lines().last(), Some("and 3 more"));
        let five = conflicts_text(&conflicts[..5], names, fronts);
        assert!(!five.contains("more"));
    }

    #[test]
    fn the_import_toast_says_what_was_added() {
        let report = ImportReport {
            decks: 1,
            cards: 2,
            review_events: 0,
            backup: None,
        };
        assert_eq!(imported_text(&report), "Imported 1 deck and 2 cards");
    }
}
