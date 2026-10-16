//! The user's own deck names and card fronts that import conflicts refer
//! to.

use rappel_core::Error;
use rappel_core::interchange::Conflict;

use super::text::{SHOWN_CONFLICTS, short_front};
use crate::state::AppState;
use crate::ui::toast::log_failure;

/// The name of the user's own deck or card that a conflict refers to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum Lookup {
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
pub(super) struct Names {
    /// `None` if the decks could not be read.
    decks: Option<Vec<(String, String)>>,
    cards: Vec<(String, Lookup)>,
}

impl Names {
    pub(super) fn read(state: &AppState, conflicts: &[Conflict]) -> Self {
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

    pub(super) fn deck(&self, id: &str) -> Lookup {
        let Some(decks) = &self.decks else {
            return Lookup::Unknown;
        };
        decks
            .iter()
            .find(|(i, _)| i == id)
            .map_or(Lookup::Deleted, |(_, name)| Lookup::Found(name.clone()))
    }

    pub(super) fn card(&self, id: &str) -> Lookup {
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
