//! Every text and small decision of the All Cards page that needs no GTK:
//! row captions, counts, toast texts, which empty state to show, what
//! Escape does. Pure, so it is unit-tested (SPEC §4.6, ADR 0036).

use rappel_core::storage::{BrowseQuery, CardStatus, StateFilter};

use crate::ui::text::{count, group_digits};
use crate::ui::time::relative_due;

/// A card's status as a short phrase: "New", "Learning", "Due now",
/// "Overdue by 2 days", "Due in 3 days", "Suspended".
pub fn status_text(status: CardStatus, now: i64) -> String {
    match status {
        CardStatus::New => "New".to_owned(),
        CardStatus::Learning(_) => "Learning".to_owned(),
        CardStatus::Suspended => "Suspended".to_owned(),
        CardStatus::Due(due) => capitalised(&relative_due(due, now)),
        CardStatus::Scheduled(due) => format!("Due {}", relative_due(due, now)),
    }
}

fn capitalised(text: &str) -> String {
    let mut chars = text.chars();
    chars
        .next()
        .map(|first| first.to_uppercase().chain(chars).collect())
        .unwrap_or_default()
}

/// The line under a card's front: "Maths · Due in 3 days · algebra, hard".
/// Compact rows (a phone) leave the tags out. `tags_line` is already one
/// plain line (`card_object::tags_line`).
pub fn caption(deck: &str, status: &str, tags_line: &str, compact: bool) -> String {
    let deck = deck.split_whitespace().collect::<Vec<_>>().join(" ");
    let mut parts = vec![deck.as_str(), status];
    if !compact && !tags_line.is_empty() {
        parts.push(tags_line);
    }
    parts.join(" · ")
}

/// What a screen reader says for a row. While choosing cards it also says
/// whether the row is chosen.
pub fn row_label(front: &str, caption: &str, selected: Option<bool>) -> String {
    let base = if caption.is_empty() {
        front.to_owned()
    } else {
        format!("{front}, {caption}")
    };
    match selected {
        Some(true) => format!("{base}, selected"),
        Some(false) => format!("{base}, not selected"),
        None => base,
    }
}

/// "1,234 cards", "1 card", "No cards".
pub fn count_text(n: usize) -> String {
    if n == 0 {
        "No cards".to_owned()
    } else {
        count(n, "card", "cards")
    }
}

/// The action bar's title: "3 selected", "None selected".
pub fn selection_title(n: usize) -> String {
    if n == 0 {
        "None selected".to_owned()
    } else {
        format!("{} selected", group_digits(n))
    }
}

/// Which way the Suspend button of the action bar goes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SuspendChoice {
    Suspend,
    Unsuspend,
}

/// Unsuspend only when every chosen card is suspended; otherwise Suspend,
/// which leaves the suspended ones as they are.
pub fn suspend_choice(suspended: impl IntoIterator<Item = bool>) -> SuspendChoice {
    let mut any = false;
    for flag in suspended {
        if !flag {
            return SuspendChoice::Suspend;
        }
        any = true;
    }
    if any {
        SuspendChoice::Unsuspend
    } else {
        SuspendChoice::Suspend
    }
}

/// What the list area shows when it has no row.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Empty {
    /// There are rows.
    No,
    /// The collection has no card at all.
    NoCards,
    /// A search or filter matches nothing.
    NoResults,
}

/// `shown` rows came back; `filtered` says whether a search text or a filter
/// was set.
pub fn empty_state(shown: usize, filtered: bool) -> Empty {
    match (shown, filtered) {
        (0, false) => Empty::NoCards,
        (0, true) => Empty::NoResults,
        _ => Empty::No,
    }
}

/// What Escape does while the focus is in the list.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EscapeAction {
    LeaveSelection,
    ClearSearch,
    Nothing,
}

/// Escape first leaves the selection mode (the mode you are in), then
/// clears the search and filters. In the search entry, the entry's own
/// `stop-search` clears the text first.
pub fn escape_action(selecting: bool, filtered: bool) -> EscapeAction {
    if selecting {
        EscapeAction::LeaveSelection
    } else if filtered {
        EscapeAction::ClearSearch
    } else {
        EscapeAction::Nothing
    }
}

/// Which entry of a filter drop-down (entry 0 is "All …") names an item:
/// the index into the `len` items, or `None` for "All …" or no selection.
pub fn filter_choice(selected: u32, len: usize) -> Option<usize> {
    let index = usize::try_from(selected).ok()?.checked_sub(1)?;
    (index < len).then_some(index)
}

/// The state drop-down's entries: Any State, New, Learning, Due, Suspended.
pub fn state_filter(selected: u32) -> Option<StateFilter> {
    match selected {
        1 => Some(StateFilter::New),
        2 => Some(StateFilter::Learning),
        3 => Some(StateFilter::Due),
        4 => Some(StateFilter::Suspended),
        _ => None,
    }
}

/// Whether `query` asks for anything (words or a filter).
pub fn is_filtered(query: &BrowseQuery) -> bool {
    !query.text.trim().is_empty()
        || query.deck_id.is_some()
        || query.tag_id.is_some()
        || query.state.is_some()
}

/// The note after an action found that a card of the list was gone.
pub fn stale_text() -> String {
    "The list was out of date; nothing was changed".to_owned()
}

/// The first entry of the deck and tag drop-downs.
pub const ALL_DECKS: &str = "All Decks";
pub const ALL_TAGS: &str = "All Tags";

/// The Suspend button's label and tooltip for `choice`.
pub fn suspend_labels(choice: SuspendChoice) -> (&'static str, &'static str) {
    match choice {
        SuspendChoice::Suspend => ("Suspend", "Suspend selected cards"),
        SuspendChoice::Unsuspend => ("Unsuspend", "Unsuspend selected cards"),
    }
}

/// The body of the Move dialog.
pub fn move_body(n: usize) -> String {
    format!("Move {} to:", count_text(n))
}

/// The body of the Remove Tag dialog.
pub fn remove_tag_body(n: usize) -> String {
    format!("Take a tag off {}:", count_text(n))
}

/// The notice when the chosen cards have no tag to take off.
pub fn no_tags_text() -> String {
    "The chosen cards have no tags".to_owned()
}

/// "3 cards" / "1 card".
fn cards(n: usize) -> String {
    count(n, "card", "cards")
}

pub fn moved_text(n: usize, deck: &str) -> String {
    format!("{} moved to {}", cards(n), one_line(deck))
}

pub fn tag_added_text(n: usize, tag: &str) -> String {
    format!("Tag “{}” added to {}", one_line(tag), cards(n))
}

pub fn tag_removed_text(n: usize, tag: &str) -> String {
    format!("Tag “{}” removed from {}", one_line(tag), cards(n))
}

pub fn suspended_text(n: usize, suspended: bool) -> String {
    let what = if suspended {
        "suspended"
    } else {
        "unsuspended"
    };
    format!("{} {what}", cards(n))
}

pub fn reset_text(n: usize) -> String {
    format!("Progress reset for {}", cards(n))
}

pub fn deleted_text(n: usize) -> String {
    format!("{} deleted", cards(n))
}

/// What an action did when every card was already as asked.
pub fn unchanged_text() -> String {
    "Nothing changed; the cards were already like that".to_owned()
}

/// The note after an Undo that could not change every card (`done` of
/// `total`), or `None` if it did.
pub fn partial_undo_text(done: usize, total: usize) -> Option<String> {
    (done < total).then(|| {
        format!(
            "{} of {} restored; the others changed since",
            group_digits(done),
            cards(total)
        )
    })
}

/// A deck or tag name on one line (names are only trimmed).
pub fn one_line(name: &str) -> String {
    name.split_whitespace().collect::<Vec<_>>().join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: i64 = 1_000_000_000_000;
    const DAY: i64 = 86_400_000;

    #[test]
    fn a_status_reads_as_a_phrase() {
        assert_eq!(status_text(CardStatus::New, NOW), "New");
        assert_eq!(status_text(CardStatus::Learning(NOW), NOW), "Learning");
        assert_eq!(status_text(CardStatus::Suspended, NOW), "Suspended");
        assert_eq!(status_text(CardStatus::Due(NOW - 1), NOW), "Due now");
        assert_eq!(
            status_text(CardStatus::Due(NOW - 2 * DAY), NOW),
            "Overdue by 2 days"
        );
        assert_eq!(
            status_text(CardStatus::Scheduled(NOW + 3 * DAY), NOW),
            "Due in 3 days"
        );
    }

    #[test]
    fn a_caption_joins_deck_status_and_tags_but_not_on_a_phone() {
        assert_eq!(
            caption("Maths", "New", "algebra, hard", false),
            "Maths · New · algebra, hard"
        );
        assert_eq!(caption("Maths", "New", "", false), "Maths · New");
        assert_eq!(caption("Maths", "New", "algebra", true), "Maths · New");
        assert_eq!(caption("a\n b", "New", "", true), "a b · New");
    }

    #[test]
    fn a_row_is_read_with_its_selection() {
        assert_eq!(
            row_label("2 + 2", "Maths · New", None),
            "2 + 2, Maths · New"
        );
        assert_eq!(row_label("2 + 2", "", None), "2 + 2");
        assert_eq!(
            row_label("2 + 2", "Maths", Some(true)),
            "2 + 2, Maths, selected"
        );
        assert_eq!(
            row_label("2 + 2", "Maths", Some(false)),
            "2 + 2, Maths, not selected"
        );
    }

    #[test]
    fn counts_and_titles() {
        assert_eq!(count_text(0), "No cards");
        assert_eq!(count_text(1), "1 card");
        assert_eq!(count_text(5_000), "5,000 cards");
        assert_eq!(selection_title(0), "None selected");
        assert_eq!(selection_title(3), "3 selected");
        assert_eq!(selection_title(1_200), "1,200 selected");
    }

    #[test]
    fn unsuspend_is_offered_only_when_every_card_is_suspended() {
        use SuspendChoice::{Suspend, Unsuspend};
        assert_eq!(suspend_choice([true, true]), Unsuspend);
        assert_eq!(suspend_choice([true, false]), Suspend);
        assert_eq!(suspend_choice([false]), Suspend);
        assert_eq!(suspend_choice([]), Suspend);
    }

    #[test]
    fn the_empty_state_depends_on_whether_anything_was_asked() {
        assert_eq!(empty_state(0, false), Empty::NoCards);
        assert_eq!(empty_state(0, true), Empty::NoResults);
        assert_eq!(empty_state(1, true), Empty::No);
        assert_eq!(empty_state(1, false), Empty::No);
    }

    #[test]
    fn escape_leaves_the_selection_before_it_clears_the_search() {
        assert_eq!(escape_action(true, true), EscapeAction::LeaveSelection);
        assert_eq!(escape_action(false, true), EscapeAction::ClearSearch);
        assert_eq!(escape_action(false, false), EscapeAction::Nothing);
    }

    #[test]
    fn filter_entries_map_to_items_and_states() {
        assert_eq!(filter_choice(0, 3), None);
        assert_eq!(filter_choice(1, 3), Some(0));
        assert_eq!(filter_choice(3, 3), Some(2));
        assert_eq!(filter_choice(4, 3), None);
        // GTK_INVALID_LIST_POSITION
        assert_eq!(filter_choice(u32::MAX, 3), None);
        assert_eq!(state_filter(0), None);
        assert_eq!(state_filter(1), Some(StateFilter::New));
        assert_eq!(state_filter(4), Some(StateFilter::Suspended));
        assert_eq!(state_filter(u32::MAX), None);
    }

    #[test]
    fn a_query_is_filtered_by_words_or_any_filter() {
        let mut q = BrowseQuery::default();
        assert!(!is_filtered(&q));
        q.text = "  ".to_owned();
        assert!(!is_filtered(&q));
        q.text = "x".to_owned();
        assert!(is_filtered(&q));
        q = BrowseQuery {
            state: Some(StateFilter::Due),
            ..BrowseQuery::default()
        };
        assert!(is_filtered(&q));
    }

    #[test]
    fn dialog_and_button_texts() {
        assert_eq!(move_body(3), "Move 3 cards to:");
        assert_eq!(remove_tag_body(1), "Take a tag off 1 card:");
        assert_eq!(
            suspend_labels(SuspendChoice::Unsuspend),
            ("Unsuspend", "Unsuspend selected cards")
        );
        assert_eq!(one_line("a\n  b"), "a b");
    }

    #[test]
    fn toasts_count_and_name() {
        assert_eq!(moved_text(3, "Maths"), "3 cards moved to Maths");
        assert_eq!(moved_text(1, "a\n b"), "1 card moved to a b");
        assert_eq!(tag_added_text(2, "hard"), "Tag “hard” added to 2 cards");
        assert_eq!(
            tag_removed_text(1, "hard"),
            "Tag “hard” removed from 1 card"
        );
        assert_eq!(suspended_text(2, true), "2 cards suspended");
        assert_eq!(suspended_text(1, false), "1 card unsuspended");
        assert_eq!(reset_text(1_000), "Progress reset for 1,000 cards");
        assert_eq!(deleted_text(1), "1 card deleted");
        assert_eq!(
            unchanged_text(),
            "Nothing changed; the cards were already like that"
        );
    }

    #[test]
    fn a_partial_undo_says_how_much_came_back() {
        assert_eq!(partial_undo_text(3, 3), None);
        assert_eq!(
            partial_undo_text(2, 3).as_deref(),
            Some("2 of 3 cards restored; the others changed since")
        );
        assert_eq!(
            partial_undo_text(0, 1).as_deref(),
            Some("0 of 1 card restored; the others changed since")
        );
    }
}
