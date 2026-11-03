//! The bulk actions of selection mode and of All Cards' row menus: what each
//! does to the cards through `AppState`, what its Undo toast says, and how
//! that Undo takes it back (SPEC §4.6, ADR 0035, ADR 0036, ADR 0065).

use rappel_core::domain::Tag;

use super::text;
use crate::dialogs::move_card;
use crate::state::AppState;
use crate::widgets::tag_entry::field::MAX_TAGS_PER_CARD;

/// What a bulk action does to the cards it is given.
#[derive(Debug, Clone)]
pub enum Bulk {
    Suspend(bool),
    Move {
        deck_id: String,
    },
    /// Move from one card's menu: the same, with a toast for one card
    /// ("Card moved"). Its `ids` hold exactly one card.
    MoveOne {
        deck_id: String,
    },
    AddTags(Vec<String>),
    RemoveTag(Tag),
    Reset,
    Delete,
}

/// Takes a bulk action back; returns how many cards it restored.
type UndoFn = Box<dyn Fn(&AppState) -> rappel_core::Result<usize>>;

/// A bulk action that was done: what to tell, how many cards it changed
/// and how to take it back. The undo returns how many cards it restored.
pub struct Done {
    /// The Undo toast's title: short, so it fits beside Undo at 360 px
    /// (ADR 0058).
    pub text: String,
    /// More to say, in a toast of its own after it (which gets Details
    /// when long): the cards Add Tags left out.
    pub note: Option<String>,
    pub total: usize,
    pub undo: UndoFn,
}

/// Does `action` on `ids`. `Err` with the notice to show if no card
/// changed: every card was already as asked, or (Add Tags) had no room.
pub fn perform(
    state: &AppState,
    ids: &[String],
    action: Bulk,
    now: i64,
) -> rappel_core::Result<Result<Done, String>> {
    let one_card = matches!(action, Bulk::MoveOne { .. });
    if one_card {
        debug_assert_eq!(ids.len(), 1, "a card menu's Move to Deck… moves one card");
    }
    let done = match action {
        Bulk::Suspend(suspended) => {
            let changed = state.set_suspended(ids, suspended)?;
            Done {
                note: None,
                text: text::suspended_text(changed.len(), suspended),
                total: changed.len(),
                undo: Box::new(move |s| s.restore_suspended(&changed, !suspended)),
            }
        }
        Bulk::Move { deck_id } | Bulk::MoveOne { deck_id } => {
            let moved = state.move_cards(ids, &deck_id)?;
            Done {
                note: None,
                text: if one_card {
                    move_card::moved_text()
                } else {
                    text::moved_text(moved.len())
                },
                total: moved.len(),
                undo: Box::new(move |s| s.unmove_cards(&moved)),
            }
        }
        Bulk::AddTags(names) => {
            let limit = MAX_TAGS_PER_CARD;
            let added = state.add_tags_to_cards(ids, &names, limit)?;
            if added.cards.is_empty() {
                return Ok(Err(if added.full.is_empty() {
                    text::unchanged_text()
                } else {
                    text::no_room_text(limit)
                }));
            }
            let links = added.links;
            Done {
                text: text::tags_added_text(added.cards.len()),
                note: text::tags_left_out_text(added.full.len(), limit),
                total: added.cards.len(),
                undo: Box::new(move |s| s.release_tags(&links)),
            }
        }
        Bulk::RemoveTag(tag) => {
            let changed = state.remove_tag_from_cards(ids, &tag.id)?;
            Done {
                note: None,
                text: text::tag_removed_text(changed.len()),
                total: changed.len(),
                undo: Box::new(move |s| s.restore_tag_on_cards(&changed, &tag.id)),
            }
        }
        Bulk::Reset => {
            let events = state.reset_cards(ids, now)?;
            if events.is_empty() {
                return Ok(Err(text::no_progress_text(ids.len())));
            }
            Done {
                note: None,
                text: text::reset_text(events.len()),
                total: events.len(),
                undo: Box::new(move |s| {
                    let skipped = s.undo_entries(&events, rappel_core::time::now_ms())?;
                    Ok(events.len().saturating_sub(skipped.len()))
                }),
            }
        }
        Bulk::Delete => {
            state.delete_cards(ids)?;
            let ids = ids.to_vec();
            Done {
                note: None,
                text: text::deleted_text(ids.len()),
                total: ids.len(),
                undo: Box::new(move |s| s.undelete_cards(&ids)),
            }
        }
    };
    Ok(if done.total > 0 {
        Ok(done)
    } else {
        Err(text::unchanged_text())
    })
}

#[cfg(test)]
pub(crate) mod tests {
    use std::time::Duration;

    use rappel_core::domain::Grade;
    use rappel_core::storage::BrowseQuery;

    use super::*;

    use crate::ui::time::DAY_MS;

    pub(crate) fn tags(names: &[&str]) -> Vec<String> {
        names.iter().map(|n| (*n).to_owned()).collect()
    }

    /// A state with decks "Maths" (algebra cards) and "Art" (one card).
    /// Returns the ids of the Maths cards.
    pub(crate) fn state_with_cards() -> (tempfile::TempDir, AppState, Vec<String>, String) {
        let (tmp, state) = crate::testing::temporary_state();
        let maths = state.create_deck("Maths").unwrap();
        let art = state.create_deck("Art").unwrap();
        let ids = ["alpha equation", "beta matrix", "gamma vector"]
            .iter()
            .map(|front| {
                state
                    .add_card_with_tags(&maths.id, front, "back", &tags(&["algebra"]))
                    .unwrap()
                    .card
                    .id
            })
            .collect();
        state
            .add_card_with_tags(&art.id, "alpha painting", "back", &[])
            .unwrap();
        (tmp, state, ids, maths.id)
    }

    #[test]
    fn every_action_is_taken_back_by_its_undo() {
        let (_tmp, state, ids, maths) = state_with_cards();
        let art = state
            .decks()
            .unwrap()
            .into_iter()
            .find(|d| d.name == "Art")
            .unwrap();
        let now = rappel_core::time::now_ms();
        let query = BrowseQuery::default();
        let rows = |state: &AppState| state.browse(&query, now).unwrap();
        let before = rows(&state);

        // Suspend and unsuspend.
        let done = perform(&state, &ids, Bulk::Suspend(true), now)
            .unwrap()
            .unwrap();
        assert_eq!((done.text.as_str(), done.total), ("3 cards suspended", 3));
        assert_eq!(rows(&state).iter().filter(|r| r.card.suspended).count(), 3);
        assert_eq!((done.undo)(&state).unwrap(), 3);
        assert_eq!(rows(&state), before);
        // Already as asked: nothing to tell.
        assert_eq!(
            perform(&state, &ids, Bulk::Suspend(false), now)
                .unwrap()
                .err()
                .as_deref(),
            Some("Nothing changed; the cards were already like that")
        );

        // Move.
        let bulk = Bulk::Move {
            deck_id: art.id.clone(),
        };
        let done = perform(&state, &ids, bulk, now).unwrap().unwrap();
        assert_eq!(done.text, "3 cards moved");
        assert_eq!(state.card(&ids[0]).unwrap().deck_id, art.id);
        assert_eq!((done.undo)(&state).unwrap(), 3);
        assert_eq!(state.card(&ids[0]).unwrap().deck_id, maths);

        // Add a tag, take a tag off.
        let done = perform(&state, &ids, Bulk::AddTags(tags(&["hard"])), now)
            .unwrap()
            .unwrap();
        assert_eq!(done.text, "3 cards tagged");
        assert_eq!(state.card_tags(&ids[1]).unwrap().len(), 2);
        assert_eq!((done.undo)(&state).unwrap(), 3);
        assert_eq!(rows(&state), before);
        // Several at once; one the cards have already is not named, and one
        // Undo takes off the others.
        let bulk = Bulk::AddTags(tags(&["Algebra", "hard", "exam"]));
        let done = perform(&state, &ids, bulk, now).unwrap().unwrap();
        assert_eq!(done.text, "3 cards tagged");
        assert_eq!(state.card_tags(&ids[1]).unwrap().len(), 3);
        assert_eq!((done.undo)(&state).unwrap(), 3);
        assert_eq!(rows(&state), before);
        let algebra = state
            .tags()
            .unwrap()
            .into_iter()
            .find(|t| t.name == "algebra")
            .unwrap();
        let done = perform(&state, &ids, Bulk::RemoveTag(algebra), now)
            .unwrap()
            .unwrap();
        assert_eq!(done.text, "3 cards untagged");
        assert_eq!(state.card_tags(&ids[1]).unwrap().len(), 0);
        assert_eq!((done.undo)(&state).unwrap(), 3);
        assert_eq!(rows(&state), before);

        // Delete.
        let done = perform(&state, &ids, Bulk::Delete, now).unwrap().unwrap();
        assert_eq!(done.text, "3 cards deleted");
        assert_eq!(rows(&state).len(), 1);
        assert_eq!((done.undo)(&state).unwrap(), 3);
        assert_eq!(rows(&state), before);
    }

    #[test]
    fn a_reset_is_undone_unless_the_card_was_studied_since() {
        let (_tmp, state, ids, _) = state_with_cards();
        let now = rappel_core::time::now_ms();
        for id in &ids[..2] {
            for at in [now - 60 * DAY_MS, now - 60 * DAY_MS + 11 * 60 * 1000] {
                state
                    .record_review(id, Grade::Pass, Duration::ZERO, at)
                    .unwrap();
            }
        }
        let done = perform(&state, &ids[..2], Bulk::Reset, now)
            .unwrap()
            .unwrap();
        assert_eq!(done.text, "2 cards reset");
        state
            .record_review(&ids[0], Grade::Pass, Duration::ZERO, now)
            .unwrap();
        // One card was studied since: only the other comes back.
        assert_eq!((done.undo)(&state).unwrap(), 1);
        assert_eq!(
            text::partial_undo_text(1, done.total).as_deref(),
            Some("1 of 2 cards restored; the others changed since")
        );
    }

    #[test]
    fn add_tags_leaves_out_cards_at_the_limit_of_the_tags_field() {
        let (_tmp, state, ids, _) = state_with_cards();
        // The first card has the field's 100 tags: "algebra" and 99 more.
        let many: Vec<String> = (1..MAX_TAGS_PER_CARD).map(|i| format!("t{i}")).collect();
        state
            .add_tags_to_cards(&ids[..1], &many, MAX_TAGS_PER_CARD)
            .unwrap();
        assert_eq!(state.card_tags(&ids[0]).unwrap().len(), MAX_TAGS_PER_CARD);
        let now = rappel_core::time::now_ms();
        let result = perform(&state, &ids[..1], Bulk::AddTags(tags(&["x"])), now);
        assert_eq!(
            result.unwrap().err().as_deref(),
            Some("No tags added; a card can have at most 100 tags")
        );
        // A tag it has already is no tag past the limit.
        let result = perform(&state, &ids[..1], Bulk::AddTags(tags(&["t1"])), now);
        assert_eq!(
            result.unwrap().err().as_deref(),
            Some("Nothing changed; the cards were already like that")
        );
        let done = perform(&state, &ids, Bulk::AddTags(tags(&["x"])), now)
            .unwrap()
            .unwrap();
        assert_eq!(done.text, "2 cards tagged");
        assert_eq!(
            done.note.as_deref(),
            Some("1 card left out. A card can have at most 100 tags")
        );
        assert_eq!(state.card_tags(&ids[0]).unwrap().len(), MAX_TAGS_PER_CARD);
        assert_eq!((done.undo)(&state).unwrap(), 2);
    }

    #[test]
    fn a_stale_list_changes_nothing() {
        let (_tmp, state, ids, _) = state_with_cards();
        state.delete_card(&ids[2]).unwrap();
        let result = perform(&state, &ids, Bulk::Suspend(true), 0);
        assert!(matches!(result, Err(rappel_core::Error::CardNotFound(_))));
        assert!(!state.card(&ids[0]).unwrap().suspended);
    }

    #[test]
    fn a_move_from_a_card_menu_says_card_moved_and_undoes() {
        crate::testing::with_widgets(|| {
            let (_tmp, state, ids, _) = state_with_cards();
            let art = state
                .decks()
                .unwrap()
                .into_iter()
                .find(|d| d.name == "Art")
                .unwrap();
            let now = rappel_core::time::now_ms();
            let done = perform(
                &state,
                &ids[..1],
                Bulk::MoveOne {
                    deck_id: art.id.clone(),
                },
                now,
            )
            .unwrap()
            .unwrap();
            assert_eq!(done.text, "Card moved");
            assert_eq!(state.card(&ids[0]).unwrap().deck_id, art.id);
            assert_eq!((done.undo)(&state).unwrap(), 1);
            // The bulk action keeps counting.
            let done = perform(&state, &ids[..1], Bulk::Move { deck_id: art.id }, now)
                .unwrap()
                .unwrap();
            assert_eq!(done.text, "1 card moved");
        });
    }
}
