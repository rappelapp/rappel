//! Every text and small decision of the All Cards page that needs no GTK:
//! row captions and labels, counts, which empty state to show and what it
//! says, and what Escape and typing in the list do. Pure, so it is
//! unit-tested (SPEC §4.6, ADR 0036). The texts of selection mode (its
//! title, the action bar, the dialogs, the Undo toasts, Space) are shared
//! with a deck's page, in `ui::selection::text`.

use rappel_core::domain::{Deck, Tag, name_key};
use rappel_core::storage::{BrowseQuery, CardStatus, StateFilter};

use crate::i18n::{fill, gettext, ngettext};
use crate::ui::text::{counted, isolate, one_line};
use crate::ui::time::relative_due;

/// A card's status as a short phrase: "New", "Learning", "Due now",
/// "Overdue by 2 days", "Due in 3 days", "Suspended".
pub fn status_text(status: CardStatus, now: i64) -> String {
    match status {
        CardStatus::New => gettext("New"),
        CardStatus::Learning(_) => gettext("Learning"),
        CardStatus::Suspended => gettext("Suspended"),
        CardStatus::Due(due) => capitalised(&relative_due(due, now)),
        CardStatus::Scheduled(due) => fill(
            // Translators: a card's status; {when} is "in 3 days".
            &gettext("Due {when}"),
            &[("when", &relative_due(due, now))],
        ),
    }
}

fn capitalised(text: &str) -> String {
    let mut chars = text.chars();
    chars
        .next()
        .map(|first| first.to_uppercase().chain(chars).collect())
        .unwrap_or_default()
}

/// The status and deck of an All Cards row: "Due in 3 days · Maths" (see
/// [`subtitle`]). The status comes first: when the line is too narrow
/// (a phone, a long deck name) it is the deck's name that ends in "…",
/// and the deck filter names it anyway.
pub fn caption(deck: &str, status: &str) -> String {
    // Isolated, so a right-to-left deck name or an open bidi override
    // cannot reorder what follows it.
    fill(
        // Translators: under a card in All Cards: {status} is "Due in 3
        // days", {deck} its deck's name.
        &gettext("{status} · {deck}"),
        &[("deck", &isolate(&one_line(deck))), ("status", status)],
    )
}

/// The second line of an All Cards row, under the front (ADR 0077): the
/// status and the deck ([`caption`]), then, when the card's back follows
/// on the same line (`before_back`; not on a narrow window, where the line
/// has room for the status and deck only), a " ·" that keeps the two
/// apart: "Due in 3 days · Maths ·". The tags are pills at the row's end
/// (`widgets::card_row`).
pub fn subtitle(deck: &str, status: &str, before_back: bool) -> String {
    let caption = caption(deck, status);
    if before_back {
        fill(
            // Translators: the start of the line under a card in All
            // Cards, before the card's back: {subtitle} is "Due in 3 days
            // · Maths".
            &gettext("{subtitle} ·"),
            &[("subtitle", &caption)],
        )
    } else {
        caption
    }
}

/// What a screen reader says for an All Cards row: the card as the deck
/// page says it (`card_row::text::row_label`, without "suspended", which
/// the status says), then the deck and the status, and, while choosing
/// cards, whether the row is chosen: "2 + 2, back: 4, deck: Maths, Due in
/// 3 days, selected".
pub fn row_label(card: &str, deck: &str, status: &str, selected: Option<bool>) -> String {
    let base = fill(
        // Translators: what a screen reader says for a card in All Cards:
        // {card} is the card ("2 + 2, back: 4"), {deck} its deck's name,
        // {status} "Due in 3 days".
        &gettext("{card}, deck: {deck}, {status}"),
        &[
            ("card", card),
            ("deck", &one_line(deck)),
            ("status", status),
        ],
    );
    crate::ui::selection::text::with_selected(base, selected)
}

/// "1,234 cards", "1 card", "No cards".
pub fn count_text(n: usize) -> String {
    if n == 0 {
        gettext("No cards")
    } else {
        cards(n)
    }
}

/// What the list area shows when it has no row.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Empty {
    /// There are rows.
    #[default]
    No,
    /// The collection has no card at all.
    NoCards,
    /// A search or filter matches nothing.
    NoResults,
    /// Every card is in an archived deck, and those are hidden (ADR 0068).
    OnlyArchived,
}

/// The status page of an [`Empty`] state: icon, title and its one
/// sentence.
pub fn empty_page(empty: Empty) -> Option<(&'static str, String, String)> {
    match empty {
        Empty::No => None,
        Empty::NoCards => Some((
            "document-new-symbolic",
            gettext("No Cards Yet"),
            gettext("Cards you add to your decks will show up here"),
        )),
        Empty::NoResults => Some((
            "edit-find-symbolic",
            gettext("No Results Found"),
            gettext("Try a different search or other filters"),
        )),
        Empty::OnlyArchived => Some((
            "edit-find-symbolic",
            gettext("Only Archived Cards"),
            gettext("The cards of archived decks are hidden"),
        )),
    }
}

/// The button of the "Only Archived Cards" state: shows them.
pub fn show_archived_label() -> String {
    gettext("_Show Archived Decks")
}

/// The button of the "No Cards Yet" state: back to the deck list, where
/// decks are made and cards added.
pub fn go_to_decks() -> String {
    gettext("_Go to Decks")
}

/// The button of the "No Results Found" state, naming what it clears:
/// "Clear Search", "Clear Filters" or "Clear Search and Filters".
pub fn clear_label(searched: bool, filtered: bool) -> String {
    match (searched, filtered) {
        (true, false) => gettext("_Clear Search"),
        (false, true) => gettext("_Clear Filters"),
        _ => gettext("_Clear Search and Filters"),
    }
}

/// Whether `query` has words, and whether it has a filter (deck, tag or
/// state), for [`clear_label`].
pub fn searched_and_filtered(query: &BrowseQuery) -> (bool, bool) {
    (
        !query.text.trim().is_empty(),
        query.deck_id.is_some() || query.tag_id.is_some() || query.state.is_some(),
    )
}

/// `shown` rows came back; `filtered` says whether a search text or a filter
/// was set, `collection_empty` whether there is no live card at all
/// (whatever the filters, archived decks included). An empty collection is
/// "No Cards Yet" even with a filter set (opened from an empty deck), so
/// the page does not say "No Results Found" first and then change its
/// mind. No row without a search or filter while there are cards means
/// they are all in archived decks, which are hidden (the archived filter is
/// not a filter here: Escape and Clear Filters leave it, ADR 0068).
pub fn empty_state(shown: usize, filtered: bool, collection_empty: bool) -> Empty {
    if shown > 0 {
        Empty::No
    } else if collection_empty {
        Empty::NoCards
    } else if filtered {
        Empty::NoResults
    } else {
        Empty::OnlyArchived
    }
}

/// Whether the filter bar shows: not while there is no card at all, when
/// there is nothing to narrow.
pub fn filters_shown(empty: Empty) -> bool {
    empty != Empty::NoCards
}

/// The entries of the "Archived Decks" drop-down, in order: hidden (the
/// default), then shown (ADR 0068).
pub fn archived_entries() -> [String; 2] {
    [
        // Translators: All Cards' filter of archived decks: their cards
        // are not listed. Short: it shares a phone's width with three
        // other filters.
        gettext("Archived Hidden"),
        // Translators: All Cards' filter of archived decks: their cards
        // are listed too.
        gettext("Archived Shown"),
    ]
}

/// Whether the cards of archived decks are listed: the drop-down shows
/// (some deck is archived) and says "Archived Shown" (entry 1).
pub fn archived_shown(filter_visible: bool, selected: u32) -> bool {
    filter_visible && selected == 1
}

/// The decks the deck filter lists: every live deck when archived decks
/// are shown, else those that are not archived, in the order given.
pub fn filter_decks(decks: &[Deck], show_archived: bool) -> Vec<Deck> {
    decks
        .iter()
        .filter(|deck| show_archived || !deck.is_archived())
        .cloned()
        .collect()
}

/// A deck's entry in the deck filter, on one line: its name, marked when
/// it is archived ("Spanish (archived)"; the name isolated, so a
/// right-to-left name cannot reorder the mark).
pub fn deck_filter_label(deck: &Deck) -> String {
    let name = one_line(&deck.name);
    if deck.is_archived() {
        fill(
            // Translators: an archived deck in All Cards' deck filter;
            // {name} is the deck's name.
            &gettext("{name} (archived)"),
            &[("name", &isolate(&name))],
        )
    } else {
        name
    }
}

/// The key that moves from the search field to the list, whatever the
/// modifiers.
pub const TO_LIST_KEY: gtk::gdk::Key = gtk::gdk::Key::Down;

/// The key `ui::selection::text::escape_action` is about, whatever the
/// modifiers.
pub const ESCAPE_KEY: gtk::gdk::Key = gtk::gdk::Key::Escape;

/// Whether a key pressed in the list or the status page goes into the
/// search field (`search_empty`: the field has no text yet):
/// - a key that types a character, without Ctrl or Alt;
/// - a dead key or Compose (`Multi_key`): the field takes the focus and
///   its input method composes the key that follows (Compose s s → ß);
/// - Backspace, when there is text to delete (GTK's own key capture
///   forwards it too).
///
/// Never while a popover (a row's menu, a drop-down's popup) is open, nor
/// while cards are being chosen (the field is hidden then). Space, Enter,
/// Tab, Escape and the movement keys stay with the focused widget, as in
/// GTK's own key capture (gtksearchentry.c, `gtk_search_entry_is_keynav`
/// and `capture_widget_key_handled`).
pub fn types_into_search(
    key: gtk::gdk::Key,
    modifiers: gtk::gdk::ModifierType,
    in_popover: bool,
    selecting: bool,
    search_empty: bool,
) -> bool {
    use gtk::gdk::{Key, ModifierType};
    use gtk::glib::translate::IntoGlib;
    if in_popover
        || selecting
        || modifiers.intersects(ModifierType::CONTROL_MASK | ModifierType::ALT_MASK)
        || matches!(
            key,
            Key::space | Key::KP_Space | Key::Return | Key::KP_Enter | Key::ISO_Enter | Key::Menu
        )
    {
        return false;
    }
    if key == Key::BackSpace {
        return !search_empty;
    }
    // The dead keys are one range of keysyms, dead_grave (0xfe50) to
    // dead_longsolidusoverlay (0xfe93) (gdk/gdkkeysyms.h).
    let dead = (Key::dead_grave.into_glib()..=Key::dead_longsolidusoverlay.into_glib())
        .contains(&key.into_glib());
    if dead || key == Key::Multi_key {
        return true;
    }
    key.to_unicode().is_some_and(|c| !c.is_control())
}

/// Which entry of a filter drop-down (entry 0 is "All …") names an item:
/// the index into the `len` items, or `None` for "All …" or no selection.
pub fn filter_choice(selected: u32, len: usize) -> Option<usize> {
    let index = usize::try_from(selected).ok()?.checked_sub(1)?;
    (index < len).then_some(index)
}

/// The tag filter's tags: the tags in use (`used`, in name-key order), and
/// the one chosen in the filter (`kept`) where it is no longer in use, at
/// its place in that order, so a choice never vanishes under the user. A
/// tag no live card has is otherwise left out: filtering by it would only
/// ever give no cards (ADR 0052).
pub fn filter_tags(used: Vec<Tag>, kept: Option<Tag>) -> Vec<Tag> {
    let mut tags = used;
    if let Some(kept) = kept
        && !tags.iter().any(|t| t.id == kept.id)
    {
        let key = name_key(&kept.name);
        let at = tags.partition_point(|t| (name_key(&t.name), &t.id) < (key.clone(), &kept.id));
        tags.insert(at, kept);
    }
    tags
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
    let (searched, filtered) = searched_and_filtered(query);
    searched || filtered
}

/// The first entry of the deck drop-down.
pub fn all_decks() -> String {
    gettext("All Decks")
}

/// The first entry of the tag drop-down.
pub fn all_tags() -> String {
    gettext("All Tags")
}

/// "3 cards" / "1 card".
fn cards(n: usize) -> String {
    // Translators: {count} is a number of cards.
    counted(&ngettext("{count} card", "{count} cards", n), n)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tag(id: &str, name: &str) -> Tag {
        Tag {
            id: id.to_owned(),
            name: name.to_owned(),
        }
    }

    #[test]
    fn the_tag_filter_keeps_a_chosen_tag_no_card_has_any_more() {
        let used = vec![tag("1", "algebra"), tag("3", "Geometry")];
        assert_eq!(filter_tags(used.clone(), None), used);
        // Still in use: listed once.
        assert_eq!(filter_tags(used.clone(), Some(tag("1", "algebra"))), used);
        // No longer in use: kept, in name-key order (case ignored).
        let kept = filter_tags(used.clone(), Some(tag("2", "Calculus")));
        let names: Vec<&str> = kept.iter().map(|t| t.name.as_str()).collect();
        assert_eq!(names, ["algebra", "Calculus", "Geometry"]);
        let last = filter_tags(used, Some(tag("4", "zeta")));
        assert_eq!(last.last().map(|t| t.id.as_str()), Some("4"));
        assert_eq!(filter_tags(Vec::new(), Some(tag("9", "x"))).len(), 1);
    }

    const NOW: i64 = 1_000_000_000_000;
    const DAY: i64 = crate::ui::time::DAY_MS;

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
    fn a_caption_says_the_deck_and_the_status() {
        assert_eq!(caption("Maths", "New"), "New · \u{2068}Maths\u{2069}");
        assert_eq!(caption("a\n b", "Due now"), "Due now · \u{2068}a b\u{2069}");
        // An override in the name ends with it.
        assert_eq!(
            caption("\u{202e}x", "New"),
            "New · \u{2068}\u{202e}x\u{2069}"
        );
    }

    #[test]
    fn the_subtitle_says_the_deck_and_the_status_before_the_back() {
        assert_eq!(
            subtitle("Maths", "Due in 3 days", false),
            "Due in 3 days · \u{2068}Maths\u{2069}"
        );
        assert_eq!(
            subtitle("Maths", "New", true),
            "New · \u{2068}Maths\u{2069} ·"
        );
        // A placeholder in the deck name stays as it is.
        assert_eq!(
            subtitle("{subtitle}", "New", true),
            "New · \u{2068}{subtitle}\u{2069} ·"
        );
    }

    #[test]
    fn a_row_is_read_with_its_deck_status_and_selection() {
        assert_eq!(
            row_label("2 + 2, back: 4", "Maths", "New", None),
            "2 + 2, back: 4, deck: Maths, New"
        );
        assert_eq!(
            row_label("2 + 2", "a\n b", "Suspended", Some(true)),
            "2 + 2, deck: a b, Suspended, selected"
        );
        assert_eq!(
            row_label("2 + 2", "Maths", "New", Some(false)),
            "2 + 2, deck: Maths, New, not selected"
        );
    }

    #[test]
    fn empty_states_have_a_title_a_sentence_and_a_button() {
        assert_eq!(empty_page(Empty::No), None);
        let (icon, title, body) = empty_page(Empty::NoCards).unwrap();
        assert_eq!(title, "No Cards Yet");
        assert_eq!(body, "Cards you add to your decks will show up here");
        assert!(icon.ends_with("-symbolic"));
        let (_, title, body) = empty_page(Empty::NoResults).unwrap();
        assert_eq!(title, "No Results Found");
        assert_eq!(body, "Try a different search or other filters");
        assert_eq!(go_to_decks(), "_Go to Decks");
    }

    #[test]
    fn the_clear_button_names_what_it_clears() {
        let mut q = BrowseQuery {
            text: "x".to_owned(),
            ..BrowseQuery::default()
        };
        assert_eq!(searched_and_filtered(&q), (true, false));
        assert_eq!(clear_label(true, false), "_Clear Search");
        q.state = Some(StateFilter::New);
        assert_eq!(searched_and_filtered(&q), (true, true));
        assert_eq!(clear_label(true, true), "_Clear Search and Filters");
        q.text = " ".to_owned();
        assert_eq!(searched_and_filtered(&q), (false, true));
        assert_eq!(clear_label(false, true), "_Clear Filters");
    }

    #[test]
    fn counts_and_titles() {
        assert_eq!(count_text(0), "No cards");
        assert_eq!(count_text(1), "1 card");
        assert_eq!(count_text(5_000), "5,000 cards");
        assert_eq!(
            crate::ui::selection::text::selection_title(0),
            "Select Cards"
        );
        assert_eq!(crate::ui::selection::text::selection_title(3), "3 Selected");
        assert_eq!(
            crate::ui::selection::text::selection_title(1_200),
            "1,200 Selected"
        );
    }

    #[test]
    fn the_empty_state_depends_on_the_collection_then_on_what_was_asked() {
        assert_eq!(empty_state(0, false, true), Empty::NoCards);
        assert_eq!(empty_state(0, true, false), Empty::NoResults);
        // Cards, but none listed and nothing asked for: they are all in
        // hidden archived decks.
        assert_eq!(empty_state(0, false, false), Empty::OnlyArchived);
        assert!(filters_shown(Empty::OnlyArchived));
        // Opened from an empty deck in an empty collection: the deck
        // filter is set, but there is nothing to find anywhere.
        assert_eq!(empty_state(0, true, true), Empty::NoCards);
        assert_eq!(empty_state(1, true, false), Empty::No);
        assert_eq!(empty_state(1, false, false), Empty::No);
        assert!(!filters_shown(Empty::NoCards));
        assert!(filters_shown(Empty::NoResults));
        assert!(filters_shown(Empty::No));
    }

    #[test]
    fn only_typing_keys_go_into_the_search() {
        use gtk::gdk::{Key, ModifierType};
        let none = ModifierType::empty();
        let types = |key, modifiers, in_popover, selecting| {
            types_into_search(key, modifiers, in_popover, selecting, true)
        };
        assert!(types(Key::a, none, false, false));
        assert!(types(Key::A, ModifierType::SHIFT_MASK, false, false));
        assert!(types(Key::eacute, none, false, false));
        assert!(types(Key::_7, none, false, false));
        // A popover open (a row's menu, the State popup), or choosing.
        assert!(!types(Key::a, none, true, false));
        assert!(!types(Key::a, none, false, true));
        // Shortcuts and keys that move or activate.
        assert!(!types(Key::a, ModifierType::CONTROL_MASK, false, false));
        assert!(!types(Key::a, ModifierType::ALT_MASK, false, false));
        for key in [
            Key::space,
            Key::Return,
            Key::KP_Enter,
            Key::Tab,
            Key::Escape,
            Key::Down,
            Key::Home,
            Key::Page_Down,
            Key::BackSpace,
            Key::Delete,
            Key::F1,
            Key::Menu,
        ] {
            assert!(!types(key, none, false, false), "{key:?}");
        }
        // Dead keys and Compose start a search, so the field's input
        // method composes what follows.
        for key in [
            Key::Multi_key,
            Key::dead_acute,
            Key::dead_grave,
            Key::dead_circumflex,
            Key::dead_longsolidusoverlay,
        ] {
            assert!(types(key, none, false, false), "{key:?}");
            assert!(!types(key, none, true, false), "{key:?} in a popover");
        }
        // Backspace only when there is text to delete.
        assert!(!types_into_search(Key::BackSpace, none, false, false, true));
        assert!(types_into_search(Key::BackSpace, none, false, false, false));
        assert!(!types_into_search(Key::BackSpace, none, false, true, false));
        assert!(!types_into_search(Key::BackSpace, none, true, false, false));
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

    fn deck(id: &str, name: &str, archived: bool) -> Deck {
        Deck {
            id: id.to_owned(),
            name: name.to_owned(),
            created_at: 0,
            updated_at: 0,
            archived_at: archived.then_some(5),
        }
    }

    #[test]
    fn archived_decks_are_listed_and_marked_only_when_shown() {
        let decks = [
            deck("1", "Maths", false),
            deck("2", "Spanish", true),
            deck("3", "Zoo", false),
        ];
        let ids = |decks: Vec<Deck>| decks.into_iter().map(|d| d.id).collect::<Vec<_>>();
        assert_eq!(ids(filter_decks(&decks, false)), ["1", "3"]);
        assert_eq!(ids(filter_decks(&decks, true)), ["1", "2", "3"]);
        assert_eq!(deck_filter_label(&decks[0]), "Maths");
        assert_eq!(
            deck_filter_label(&decks[1]),
            "\u{2068}Spanish\u{2069} (archived)"
        );
        // One line, and braces in a name stay as they are.
        assert_eq!(
            deck_filter_label(&deck("4", "a\n {name}", true)),
            "\u{2068}a {name}\u{2069} (archived)"
        );
        assert_eq!(archived_entries(), ["Archived Hidden", "Archived Shown"]);
        assert!(archived_shown(true, 1));
        assert!(!archived_shown(true, 0));
        // Hidden while no deck is archived: nothing to show.
        assert!(!archived_shown(false, 1));
        let (_, title, body) = empty_page(Empty::OnlyArchived).unwrap();
        assert_eq!(title, "Only Archived Cards");
        assert_eq!(body, "The cards of archived decks are hidden");
        assert_eq!(show_archived_label(), "_Show Archived Decks");
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
    fn statuses_and_rows_read_in_french() {
        crate::i18n::tests::in_french(|| {
            assert_eq!(status_text(CardStatus::New, NOW), "Nouvelle");
            assert_eq!(
                status_text(CardStatus::Due(NOW - 1), NOW),
                "À revoir maintenant"
            );
            assert_eq!(
                status_text(
                    CardStatus::Scheduled(NOW + 3 * crate::ui::time::DAY_MS),
                    NOW
                ),
                "À revoir dans 3\u{a0}jours"
            );
            assert_eq!(count_text(5_000), "5\u{202f}000 cartes");
            assert_eq!(count_text(0), "Aucune carte");
            assert_eq!(
                crate::ui::selection::text::moved_text(1),
                "1 carte déplacée"
            );
        });
    }
}
