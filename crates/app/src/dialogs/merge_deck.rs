//! Merge Into… from a deck's ⋮ menu, on the deck list and on the deck page
//! (#103, ADR 0066): which decks it offers, the dialog that picks one and
//! says what the merge does, and the Undo toast. Layout in
//! `data/merge_deck_dialog.blp`. The merge and its undo are core's
//! `merge_deck` and `unmerge_deck` through `AppState`; where each page puts
//! the focus afterwards is its own.

use adw::prelude::*;
use gtk::glib;
use rappel_core::Error;
use rappel_core::domain::Deck;
use rappel_core::storage::Merged;

use crate::i18n::{fill, gettext, ngettext};
use crate::state::AppState;
use crate::ui::text::{counted, not_restored, with_error};
use crate::ui::toast::{Message, info_toast, show_error, show_toast_from, undo_toast};
use crate::ui::{dialog, dropdown};

const FILE: &str = "merge_deck_dialog.ui";

/// The decks deck `source` can be merged into: every deck but itself and
/// the archived ones (core refuses those, ADR 0068), in the order given.
/// The one place that decides it, for both menus.
/// The same decks as Move to Deck… offers a card of `source`
/// (`move_card::targets`), so the two cannot drift.
pub fn targets<'a>(decks: &'a [Deck], source: &str) -> Vec<&'a Deck> {
    super::move_card::targets(decks, source)
}

/// Whether Merge Into… is enabled for deck `source`: when there is a deck
/// to merge it into. Otherwise the dialog would have nothing to choose.
pub fn offered(decks: &[Deck], source: &str) -> bool {
    !targets(decks, source).is_empty()
}

/// What the dialog says the merge does. `count` is the source's live
/// cards (suspended ones included): the deleted ones move too, but the
/// user no longer sees them, so they are not counted (ADR 0066).
pub fn body(count: usize, source: &str, target: &str) -> String {
    let template = if count == 0 {
        // Translators: the text of the Merge Deck dialog when the deck has
        // no cards; {source} is the deck being merged, {target} the deck
        // chosen to merge it into. Deck names are shown in quotes.
        gettext("“{source}” has no cards to move into “{target}”. Remove “{source}”?")
    } else {
        // Translators: the text of the Merge Deck dialog; {count} is a
        // number of cards, {source} the deck being merged, {target} the
        // deck chosen to merge it into. Deck names are shown in quotes.
        counted(
            &ngettext(
                "Move {count} card from “{source}” into “{target}” and remove “{source}”?",
                "Move {count} cards from “{source}” into “{target}” and remove “{source}”?",
                count,
            ),
            count,
        )
    };
    fill(&template, &[("source", source), ("target", target)])
}

/// The Undo toast's title. Short, so it fits beside Undo at 360 px
/// (ADR 0058).
pub fn merged_text() -> String {
    gettext("Decks merged")
}

/// What to tell when there is no deck left to merge into (the last other
/// deck was deleted since the menu was shown).
pub fn no_other_deck_text() -> String {
    gettext("No other deck to merge it into")
}

/// The dialog and its drop-down; `Err` with the first missing object's id,
/// after logging every missing one.
pub(crate) fn layout() -> Result<(adw::AlertDialog, gtk::DropDown), &'static str> {
    let builder = dialog::builder(FILE);
    let (alert, dropdown) = (
        dialog::object(&builder, FILE, "merge_deck_dialog"),
        dialog::object(&builder, FILE, "merge_target_dropdown"),
    );
    Ok((alert?, dropdown?))
}

/// The dialog for merging `source`, with `count` live cards, into one of
/// `targets` (not empty), the first one chosen. The body follows the
/// choice. The drop-down has the focus once the dialog shows, at any
/// window size (`dialog::focus_when_shown`); Cancel is the default
/// response, so Enter there opens the list and never merges.
fn prepare(
    source: &Deck,
    count: usize,
    targets: &[Deck],
) -> Option<(adw::AlertDialog, gtk::DropDown)> {
    let (alert, choice) = layout().ok()?;
    let names: Vec<String> = targets.iter().map(|deck| deck.name.clone()).collect();
    dropdown::set_labels(&choice, &names);
    // A long name ends in "…" on the button; the list shows it whole.
    dropdown::shrinkable(&choice);
    // Translators: the accessible name of the list of decks in the Merge
    // Deck dialog.
    let label = gettext("Merge Into");
    choice.update_property(&[gtk::accessible::Property::Label(&label)]);
    let source_name = source.name.clone();
    let update = glib::clone!(
        #[weak]
        alert,
        move |choice: &gtk::DropDown| {
            let target = usize::try_from(choice.selected())
                .ok()
                .and_then(|i| names.get(i));
            if let Some(target) = target {
                alert.set_body(&body(count, &source_name, target));
            }
            alert.set_response_enabled("merge", target.is_some());
        }
    );
    update(&choice);
    choice.connect_selected_notify(update);
    dialog::focus_when_shown(&alert, &choice);
    Some((alert, choice))
}

/// Asks which of `targets` to merge `source` (with `count` live cards)
/// into, over `parent`, and returns the deck chosen, or `None` if
/// cancelled. `on_closed` is called once the dialog has closed: `AdwDialog`
/// gives the focus back to the widget that had it then, so a page that
/// moves the focus because of the answer does it again there (as
/// `browser_dialogs::choose_closing`).
async fn choose_target(
    parent: &impl IsA<gtk::Widget>,
    source: &Deck,
    count: usize,
    targets: Vec<Deck>,
    on_closed: impl Fn() + 'static,
) -> Option<Deck> {
    let (alert, choice) = prepare(source, count, &targets)?;
    alert.connect_closed(move |_| on_closed());
    let response = alert.choose_future(Some(parent)).await;
    if response != "merge" {
        return None;
    }
    let chosen = usize::try_from(choice.selected()).ok()?;
    targets.into_iter().nth(chosen)
}

/// A merge made from a menu: both decks as they were, for the texts of
/// its Undo, and what core needs to undo it.
#[derive(Debug, Clone)]
pub struct Done {
    /// The deck merged, now deleted.
    pub source: Deck,
    /// The deck it went into.
    pub target: Deck,
    /// What `unmerge_deck` undoes.
    pub merged: Merged,
}

/// Merge Into… for deck `source_id`, over `page`: reads the decks and the
/// card count, asks for the target, and merges. Returns the merge, or
/// `None` if there was nothing to do, the user cancelled, or it failed
/// (after a toast). `on_closed` is as in [`choose_target`].
pub async fn run(
    page: &impl IsA<gtk::Widget>,
    state: &AppState,
    source_id: &str,
    on_closed: impl Fn() + 'static,
) -> Option<Done> {
    let read = state.decks().and_then(|decks| {
        let count = state.card_count(source_id)?;
        Ok((decks, count))
    });
    let (decks, count) = match read {
        Ok(read) => read,
        // Gone meanwhile (another window): its list is rebuilt by that.
        Err(rappel_core::Error::DeckNotFound(_)) => return None,
        Err(err) => {
            // Translators: a toast: what is before ":" is its title, the rest is behind Details. {error} is why, in English.
            let message = with_error(&gettext("The decks could not be read: {error}"), &err);
            show_error(page, "reading the decks", &message);
            return None;
        }
    };
    let source = decks.iter().find(|deck| deck.id == source_id)?.clone();
    let targets: Vec<Deck> = targets(&decks, source_id).into_iter().cloned().collect();
    if targets.is_empty() {
        show_toast_from(page, info_toast(&no_other_deck_text()));
        return None;
    }
    let target = choose_target(page, &source, count, targets, on_closed).await?;
    match state.merge_deck(&source.id, &target.id) {
        Ok(merged) => Some(Done {
            source,
            target,
            merged,
        }),
        Err(err) => {
            // Translators: a toast: what is before ":" is its title, the rest is behind Details. {error} is why, in English.
            let message = with_error(&gettext("The decks could not be merged: {error}"), &err);
            show_error(page, "merging decks", &message);
            None
        }
    }
}

/// The toast "Decks merged" with Undo. Undo undoes `done` and gives
/// `after` the result (the source deck, live again) with `done`; it keeps
/// the state, not a page, so it works after the page was left.
pub fn merged_toast(
    state: &AppState,
    done: Done,
    after: impl Fn(rappel_core::Result<Deck>, &Done) + 'static,
) -> adw::Toast {
    let state = state.clone();
    undo_toast(&merged_text(), move || {
        after(state.unmerge_deck(&done.merged), &done);
    })
}

/// What to tell when the Undo of merging `source` into `target` failed
/// with `err`, in words where core's error would only give an id: the
/// deck merged into was deleted since (its delete is undone first), the
/// merged deck was deleted again, or another deck took its name. Its
/// title is decided on the template, so a ":" in a deck's name never
/// splits it ([`Message`]).
pub fn not_restored_text(source: &Deck, target: &Deck, err: &Error) -> Message {
    let name = source.name.as_str();
    match err {
        Error::DeckNotFound(id) if *id == target.id => Message::from_template(
            // Translators: a toast: what is before ":" is its title, the
            // rest is behind Details. {name} is the deck that was merged,
            // {target} the deck it was merged into, which was deleted since.
            &gettext("“{name}” could not be restored: “{target}” was deleted since."),
            &[("name", name), ("target", &target.name)],
        ),
        Error::DeckNotFound(_) => Message::from_template(
            // Translators: a toast: what is before ":" is its title, the
            // rest is behind Details. {name} is a deck's name.
            &gettext("“{name}” could not be restored: it was deleted again since."),
            &[("name", name)],
        ),
        Error::DuplicateDeckName(_) => Message::from_template(
            // Translators: a toast: what is before ":" is its title, the
            // rest is behind Details. {name} is a deck's name.
            &gettext("“{name}” could not be restored: another deck has its name now."),
            &[("name", name)],
        ),
        Error::ArchivedDeckName(_) => Message::from_template(
            // Translators: a toast: what is before ":" is its title, the
            // rest is behind Details. {name} is a deck's name.
            &gettext("“{name}” could not be restored: an archived deck has its name now."),
            &[("name", name)],
        ),
        err => not_restored(name, err),
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;

    fn deck(id: &str) -> Deck {
        Deck {
            id: id.to_owned(),
            name: format!("Deck {id}"),
            created_at: 0,
            updated_at: 0,
            archived_at: None,
        }
    }

    #[test]
    fn a_deck_is_never_offered_as_its_own_target() {
        let decks = [deck("a"), deck("b"), deck("c")];
        let ids = |source| -> Vec<&str> {
            targets(&decks, source)
                .into_iter()
                .map(|d| d.id.as_str())
                .collect()
        };
        assert_eq!(ids("a"), ["b", "c"]);
        assert_eq!(ids("c"), ["a", "b"]);
    }

    #[test]
    fn the_entry_is_enabled_only_with_another_deck() {
        assert!(!offered(&[], "a"));
        assert!(!offered(&[deck("a")], "a"));
        assert!(offered(&[deck("a"), deck("b")], "a"));
    }

    /// Archived decks are never offered (ADR 0068): with only archived
    /// others, the entry is disabled.
    #[test]
    fn archived_decks_are_not_targets() {
        let archived = |id| Deck {
            archived_at: Some(1),
            ..deck(id)
        };
        let decks = [deck("a"), archived("b"), deck("c")];
        let ids: Vec<&str> = targets(&decks, "a")
            .into_iter()
            .map(|d| d.id.as_str())
            .collect();
        assert_eq!(ids, ["c"]);
        assert!(!offered(&[deck("a"), archived("b")], "a"));
    }

    #[test]
    fn the_body_counts_the_cards_and_names_both_decks() {
        assert_eq!(
            body(12, "Spanish", "Languages"),
            "Move 12 cards from “Spanish” into “Languages” and remove “Spanish”?"
        );
        assert_eq!(
            body(1, "Spanish", "Languages"),
            "Move 1 card from “Spanish” into “Languages” and remove “Spanish”?"
        );
        assert_eq!(
            body(1_200, "A", "B"),
            "Move 1,200 cards from “A” into “B” and remove “A”?"
        );
        assert_eq!(
            body(0, "Spanish", "Languages"),
            "“Spanish” has no cards to move into “Languages”. Remove “Spanish”?"
        );
        // A name is never read for placeholders.
        assert_eq!(
            body(2, "{target}", "{count}"),
            "Move 2 cards from “{target}” into “{count}” and remove “{target}”?"
        );
    }

    #[test]
    fn a_failed_undo_says_why_in_words() {
        let mut source = deck("s");
        source.name = "Spanish".to_owned();
        let mut target = deck("t");
        target.name = "Languages".to_owned();
        assert_eq!(
            not_restored_text(&source, &target, &Error::DeckNotFound("t".to_owned())).text,
            "“Spanish” could not be restored: “Languages” was deleted since."
        );
        assert_eq!(
            not_restored_text(&source, &target, &Error::DeckNotFound("s".to_owned())).text,
            "“Spanish” could not be restored: it was deleted again since."
        );
        assert_eq!(
            not_restored_text(
                &source,
                &target,
                &Error::DuplicateDeckName("Spanish".to_owned())
            )
            .text,
            "“Spanish” could not be restored: another deck has its name now."
        );
        assert_eq!(
            not_restored_text(
                &source,
                &target,
                &Error::ArchivedDeckName("Spanish".to_owned())
            )
            .text,
            "“Spanish” could not be restored: an archived deck has its name now."
        );
        assert_eq!(
            not_restored_text(&source, &target, &Error::EmptyDeckName).text,
            "“Spanish” could not be restored: a deck name cannot be empty"
        );
        // No id is ever shown, and names are never read for placeholders.
        target.name = "{name}".to_owned();
        let text = not_restored_text(&source, &target, &Error::DeckNotFound("t".to_owned())).text;
        assert!(text.contains("“{name}” was deleted") && !text.contains(" t "));
        // A ":" in the merged deck's name never ends the title: the title
        // is the template's first clause, too long here, so the fallback,
        // and the details are the whole text (the bug: "“Spanish" as the
        // title and "Verbs” could not be restored…" behind Details).
        source.name = "Spanish: verbs".to_owned();
        target.name = "Languages".to_owned();
        for err in [
            Error::DeckNotFound("t".to_owned()),
            Error::DeckNotFound("s".to_owned()),
            Error::DuplicateDeckName("x".to_owned()),
            Error::ArchivedDeckName("x".to_owned()),
            Error::EmptyDeckName,
        ] {
            let message = not_restored_text(&source, &target, &err);
            assert!(
                message
                    .text
                    .starts_with("“Spanish: verbs” could not be restored: ")
            );
            assert_eq!(
                message.shown,
                crate::ui::toast::ToastText::Detailed {
                    title: crate::ui::toast::fallback_title(),
                    details: message.text.clone(),
                },
                "{err:?}"
            );
        }
    }

    #[test]
    fn the_texts_fit_a_toast() {
        use crate::ui::toast::{TOAST_FITS, TOAST_FITS_WITH_BUTTON};
        assert!(merged_text().chars().count() <= TOAST_FITS_WITH_BUTTON);
        assert!(no_other_deck_text().chars().count() <= TOAST_FITS);
    }

    /// The dialog: Cancel is the default and closes it, Merge is suggested,
    /// the body follows the chosen deck, and the drop-down, not a
    /// response, has the focus at every size, so Enter or Space never
    /// merges.
    #[test]
    fn the_dialog_defaults_to_cancel_and_focuses_the_choice_at_every_size() {
        crate::testing::with_widgets(|| {
            let source = deck("s");
            let targets = [deck("a"), deck("b")];
            for (w, h) in [(1000, 720), (360, 640), (640, 360)] {
                let window = adw::Window::new();
                window.set_default_size(w, h);
                window.present();
                crate::testing::settle_until(|| window.is_mapped(), Duration::from_secs(5));
                let (alert, choice) = prepare(&source, 3, &targets).unwrap();
                assert_eq!(alert.default_response().as_deref(), Some("cancel"));
                assert_eq!(alert.close_response(), "cancel");
                assert_eq!(
                    alert.response_appearance("merge"),
                    adw::ResponseAppearance::Suggested
                );
                assert_eq!(
                    alert.body(),
                    "Move 3 cards from “Deck s” into “Deck a” and remove “Deck s”?"
                );
                choice.set_selected(1);
                assert_eq!(
                    alert.body(),
                    "Move 3 cards from “Deck s” into “Deck b” and remove “Deck s”?"
                );
                alert.present(Some(&window));
                let focused = || {
                    alert
                        .focus()
                        .is_some_and(|f| f == choice || f.is_ancestor(&choice))
                };
                assert!(
                    crate::testing::settle_until(focused, Duration::from_secs(5)),
                    "at {w} × {h}"
                );
                crate::testing::settle_until(|| false, Duration::from_millis(300));
                assert!(focused(), "at {w} × {h}, later");
                alert.force_close();
                window.close();
            }
        });
    }
}
