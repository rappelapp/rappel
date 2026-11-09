//! What a card row of the deck page says to a screen reader, which of its
//! menu items it offers, and which tags it shows as pills: the tags every
//! listed card shares are said once above the list, and a row shows only
//! its others (ADR 0077). Pure, so it is unit-tested (ADR 0042).

use std::borrow::Cow;
use std::collections::HashSet;

use rappel_core::domain::Tag;

use crate::i18n::{fill, gettext, ngettext};
use crate::ui::rows::menu_text;
use crate::ui::text::{MAX_NAMED, isolate, one_line};
use crate::widgets::tag_pills::text::spoken;

/// The front as a screen reader names the card, in the row's label and its
/// menu button's ("Actions for …"): its preview, or "empty front" when it
/// has no text (e.g. a side that is only an empty formula).
pub fn spoken_front(front: &str) -> String {
    if front.is_empty() {
        gettext("empty front")
    } else {
        front.to_owned()
    }
}

/// The accessible label of a card row: the front, the back, every tag
/// (the ones folded into "+N" too) and whether the card is suspended, so a
/// screen reader says what the row shows: "2 + 2, back: 4, tags: maths,
/// easy, suspended". The previews are already one plain line each.
pub fn row_label(front: &str, back: &str, tags: &[String], suspended: bool) -> String {
    let mut parts = vec![spoken_front(front)];
    if !back.is_empty() {
        // Translators: a card's back for screen readers ("back: 4").
        parts.push(fill(&gettext("back: {back}"), &[("back", back)]));
    }
    parts.extend(spoken(tags));
    if suspended {
        parts.push(gettext("suspended"));
    }
    parts.join(&crate::ui::text::list_separator())
}

/// The tags every card of a list shares, said once above the list (ADR
/// 0077): the tags of `cards` (each card's tags) that are on every one, in
/// the first card's order, each once. None for a list of fewer than two
/// cards, where "on every card" would say nothing. Stops reading as soon
/// as no tag is left, so a large list without shared tags costs little
/// ([`SharedTags`], which the pages use one item at a time).
#[cfg(test)]
pub fn shared_tags<'a>(cards: impl IntoIterator<Item = &'a [Tag]>) -> Vec<Tag> {
    let mut shared = SharedTags::default();
    for tags in cards {
        if !shared.add(tags) {
            break;
        }
    }
    shared.finish()
}

/// [`shared_tags`] one card at a time, so a caller can read each card's
/// tags only while it adds them, and stop when [`Self::add`] says no tag
/// is left. Each card costs time in its number of tags (sets of ids), so a
/// list costs time in its total number of tags.
#[derive(Debug, Default)]
pub struct SharedTags {
    /// The first card's tags still on every card, in its order.
    kept: Vec<Tag>,
    cards: usize,
}

impl SharedTags {
    /// Adds a card with `tags`; false once no tag is shared any more (the
    /// cards after it change nothing).
    pub fn add(&mut self, tags: &[Tag]) -> bool {
        if self.cards == 0 {
            let mut seen = HashSet::with_capacity(tags.len());
            self.kept = tags
                .iter()
                .filter(|t| seen.insert(t.id.as_str()))
                .cloned()
                .collect();
        } else {
            let ids: HashSet<&str> = tags.iter().map(|t| t.id.as_str()).collect();
            self.kept.retain(|kept| ids.contains(kept.id.as_str()));
        }
        self.cards += 1;
        !self.kept.is_empty()
    }

    /// The shared tags: none for fewer than two cards.
    pub fn finish(self) -> Vec<Tag> {
        if self.cards < 2 {
            Vec::new()
        } else {
            self.kept
        }
    }
}

/// The pill names a row shows for a card with `names` (its tag names,
/// one line each) and `tags` (the same tags with their ids): the ones whose
/// id is not in `shared` (the list's [`shared_tags`], said above it). The
/// names as they are, without a copy, while nothing is shared.
pub fn other_tags<'a>(names: &'a [String], tags: &[Tag], shared: &[String]) -> Cow<'a, [String]> {
    if shared.is_empty() {
        return Cow::Borrowed(names);
    }
    let shared: HashSet<&str> = shared.iter().map(String::as_str).collect();
    Cow::Owned(
        names
            .iter()
            .zip(tags)
            .filter(|(_, tag)| !shared.contains(tag.id.as_str()))
            .map(|(name, _)| name.clone())
            .collect(),
    )
}

/// The row actions that depend on the card, each with whether it is
/// enabled: the one of Suspend and Unsuspend that changes something, and
/// Reset Progress unless the card is already new, where a reset would only
/// add a log entry that changes nothing.
pub fn row_actions(suspended: bool, is_new: bool) -> [(&'static str, bool); 3] {
    [
        ("row.suspend", !suspended),
        ("row.unsuspend", suspended),
        ("row.reset", !is_new),
    ]
}

/// "Study Cards Tagged “verbs”": the entry that starts a session over the
/// cards carrying a tag (ADR 0069), in a row's menu and on All Cards. For
/// the eye: the name is isolated (`ui::text::isolate`).
pub fn study_tag_label(name: &str) -> String {
    // Translators: starts a study session over the cards that carry a tag,
    // in every deck; {tag} is the tag's name.
    fill(
        &gettext("Study Cards Tagged “{tag}”"),
        &[("tag", &isolate(&one_line(name)))],
    )
}

/// "Study Cards Tagged “verbs” in Every Deck": All Cards' Study button,
/// whose session is the tag's in every deck that is not archived, even
/// while the deck filter names one deck (ADR 0069). For the eye, with
/// the name isolated; `spoken` gives it as it is, for a screen reader.
pub fn study_tag_everywhere(name: &str, spoken: bool) -> String {
    let name = one_line(name);
    // Translators: All Cards' button that starts a study session over the
    // cards that carry a tag, in every deck, whatever deck the filter
    // shows; {tag} is the tag's name.
    fill(
        &gettext("Study Cards Tagged “{tag}” in Every Deck"),
        &[("tag", &if spoken { name } else { isolate(&name) })],
    )
}

/// How many tags a row's menu lists as entries of their own; with more,
/// they go in a submenu.
pub const TAG_ITEMS: usize = 3;

/// A row menu's entries for studying the card's tags: each a menu label
/// (mnemonics escaped, `ui::rows::menu_text`) and the tag's id.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TagMenu {
    /// No tag: nothing.
    None,
    /// Up to [`TAG_ITEMS`] tags: "Study Cards Tagged “verbs”" each.
    Items(Vec<(String, String)>),
    /// More: a submenu "Study Cards Tagged" listing the names, at most
    /// [`MAX_NAMED`] (a card can have thousands, ADR 0050; the others are
    /// in All Cards' tag filter).
    Submenu(String, Vec<(String, String)>),
}

/// The [`TagMenu`] of a card with `tags` (in the order they are shown).
pub fn tag_menu(tags: &[Tag]) -> TagMenu {
    if tags.is_empty() {
        TagMenu::None
    } else if tags.len() <= TAG_ITEMS {
        TagMenu::Items(
            tags.iter()
                .map(|t| (menu_text(&study_tag_label(&t.name)), t.id.clone()))
                .collect(),
        )
    } else {
        TagMenu::Submenu(
            // Translators: a submenu listing a card's or a deck's tags;
            // choosing one studies the cards that carry it, in every deck.
            gettext("Study Cards _Tagged"),
            tags.iter()
                .take(MAX_NAMED)
                .map(|t| (menu_text(&isolate(&one_line(&t.name))), t.id.clone()))
                .collect(),
        )
    }
}

/// The Deck Menu's "Study Cards Tagged" submenu (ADR 0069): its label,
/// the deck's tags as entries (menu label, tag id), and, past
/// [`MAX_NAMED`], an entry saying how many more there are, which opens
/// All Cards (where the tag filter lists them all).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeckTagMenu {
    pub label: String,
    pub items: Vec<(String, String)>,
    pub more: Option<String>,
}

/// The [`DeckTagMenu`] of a deck whose live cards have `tags` (repeats
/// allowed, in any order): each tag once, ordered by name key then id as
/// the store orders them; `None` without a tag.
pub fn deck_tag_menu<'a>(tags: impl IntoIterator<Item = &'a Tag>) -> Option<DeckTagMenu> {
    let mut unique: Vec<&Tag> = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for tag in tags {
        if seen.insert(tag.id.as_str()) {
            unique.push(tag);
        }
    }
    if unique.is_empty() {
        return None;
    }
    unique.sort_by_cached_key(|t| (rappel_core::domain::name_key(&t.name), t.id.clone()));
    let hidden = unique.len().saturating_sub(MAX_NAMED);
    Some(DeckTagMenu {
        // Translators: a submenu listing a card's or a deck's tags;
        // choosing one studies the cards that carry it, in every deck.
        label: gettext("Study Cards _Tagged"),
        // No mnemonics in the names: they are the user's, and may repeat
        // letters; the underscores they have are kept as text.
        items: unique
            .iter()
            .take(MAX_NAMED)
            .map(|t| (menu_text(&isolate(&one_line(&t.name))), t.id.clone()))
            .collect(),
        more: (hidden > 0).then(|| {
            crate::ui::text::counted(
                // Translators: the last entry of the Deck Menu's list of
                // tags when it is too long to show; it opens All Cards,
                // whose tag filter lists every tag.
                &ngettext(
                    "{count} More Tag in All Cards…",
                    "{count} More Tags in All Cards…",
                    hidden,
                ),
                hidden,
            )
        }),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(list: &[&str]) -> Vec<String> {
        list.iter().map(|&n| n.to_owned()).collect()
    }

    #[test]
    fn a_row_is_read_as_it_shows() {
        assert_eq!(row_label("2 + 2", "", &[], false), "2 + 2");
        assert_eq!(row_label("2 + 2", "4", &[], false), "2 + 2, back: 4");
        assert_eq!(
            row_label("2 + 2", "4", &names(&["maths", "easy"]), false),
            "2 + 2, back: 4, tags: maths, easy"
        );
        assert_eq!(
            row_label("2 + 2", "", &names(&["maths"]), true),
            "2 + 2, tags: maths, suspended"
        );
        // A front with no text (e.g. only an empty formula) still says
        // something.
        assert_eq!(row_label("", "4", &[], false), "empty front, back: 4");
    }

    #[test]
    fn every_tag_is_read_even_past_the_pills_shown() {
        let tags = names(&["a", "b", "c", "d", "e"]);
        assert_eq!(
            row_label("Front", "", &tags, false),
            "Front, tags: a, b, c, d, e"
        );
    }

    fn tag(id: &str, name: &str) -> Tag {
        Tag {
            id: id.to_owned(),
            name: name.to_owned(),
        }
    }

    #[test]
    fn the_study_entry_names_the_tag_on_one_line() {
        assert_eq!(
            study_tag_label("verbs"),
            "Study Cards Tagged “\u{2068}verbs\u{2069}”"
        );
        assert_eq!(
            study_tag_label("irregular\nverbs"),
            "Study Cards Tagged “\u{2068}irregular verbs\u{2069}”"
        );
        // A placeholder in the name stays as it is.
        assert_eq!(
            study_tag_label("{tag}"),
            "Study Cards Tagged “\u{2068}{tag}\u{2069}”"
        );
    }

    #[test]
    fn the_all_cards_button_says_every_deck() {
        assert_eq!(
            study_tag_everywhere("verbs", false),
            "Study Cards Tagged “\u{2068}verbs\u{2069}” in Every Deck"
        );
        assert_eq!(
            study_tag_everywhere("a\nb", true),
            "Study Cards Tagged “a b” in Every Deck"
        );
    }

    #[test]
    fn the_deck_menu_lists_each_tag_once_in_order_and_counts_the_rest() {
        assert_eq!(deck_tag_menu(&[]), None);
        let tags = [
            tag("2", "b_x"),
            tag("1", "Alpha"),
            tag("2", "b_x"),
            tag("3", "éclair"),
        ];
        let menu = deck_tag_menu(&tags).unwrap();
        assert_eq!(menu.label, "Study Cards _Tagged");
        assert_eq!(
            menu.items,
            [
                ("\u{2068}Alpha\u{2069}".to_owned(), "1".to_owned()),
                ("\u{2068}b__x\u{2069}".to_owned(), "2".to_owned()),
                ("\u{2068}éclair\u{2069}".to_owned(), "3".to_owned()),
            ]
        );
        assert_eq!(menu.more, None);
        let many: Vec<Tag> = (0..MAX_NAMED + 3)
            .map(|i| tag(&format!("{i:03}"), &format!("t{i:03}")))
            .collect();
        let menu = deck_tag_menu(&many).unwrap();
        assert_eq!(menu.items.len(), MAX_NAMED);
        assert_eq!(menu.more.as_deref(), Some("3 More Tags in All Cards…"));
        let one_more: Vec<Tag> = many[..=MAX_NAMED].to_vec();
        assert_eq!(
            deck_tag_menu(&one_more).unwrap().more.as_deref(),
            Some("1 More Tag in All Cards…")
        );
    }

    #[test]
    fn a_few_tags_are_entries_and_more_a_submenu() {
        assert_eq!(tag_menu(&[]), TagMenu::None);
        let few = [tag("1", "verbs"), tag("2", "my_tag")];
        assert_eq!(
            tag_menu(&few),
            TagMenu::Items(vec![
                (
                    "Study Cards Tagged “\u{2068}verbs\u{2069}”".to_owned(),
                    "1".to_owned()
                ),
                // An underscore is not a mnemonic.
                (
                    "Study Cards Tagged “\u{2068}my__tag\u{2069}”".to_owned(),
                    "2".to_owned()
                ),
            ])
        );
        let three: Vec<Tag> = (0..TAG_ITEMS).map(|i| tag(&i.to_string(), "t")).collect();
        assert!(matches!(tag_menu(&three), TagMenu::Items(items) if items.len() == TAG_ITEMS));
        let many: Vec<Tag> = (0..1_000)
            .map(|i| tag(&i.to_string(), &format!("t_{i}")))
            .collect();
        let TagMenu::Submenu(label, items) = tag_menu(&many) else {
            panic!("a submenu");
        };
        assert_eq!(label, "Study Cards _Tagged");
        assert_eq!(items.len(), MAX_NAMED);
        assert_eq!(
            items[0],
            ("\u{2068}t__0\u{2069}".to_owned(), "0".to_owned())
        );
    }

    fn tags(list: &[&str]) -> Vec<Tag> {
        list.iter().map(|&n| tag(&format!("id-{n}"), n)).collect()
    }

    fn ids(tags: &[Tag]) -> Vec<String> {
        tags.iter().map(|t| t.id.clone()).collect()
    }

    #[test]
    fn the_shared_tags_are_on_every_card_in_the_first_cards_order() {
        let cards = [
            tags(&["Probabilités", "Définition", "SIC3101", "Analyse"]),
            tags(&["SIC3101", "Définition", "Probabilités"]),
            tags(&["Définition", "Probabilités", "SIC3101", "MAT3101"]),
        ];
        let shared = shared_tags(cards.iter().map(Vec::as_slice));
        assert_eq!(
            ids(&shared),
            ["id-Probabilités", "id-Définition", "id-SIC3101"]
        );
        // One card without tags: nothing is shared.
        let with_bare = [tags(&["a", "b"]), Vec::new(), tags(&["a"])];
        assert_eq!(
            shared_tags(with_bare.iter().map(Vec::as_slice)),
            Vec::<Tag>::new()
        );
        // One card alone, or none: nothing is said above the list.
        assert_eq!(shared_tags([tags(&["a"]).as_slice()]), Vec::<Tag>::new());
        assert_eq!(shared_tags(std::iter::empty::<&[Tag]>()), Vec::<Tag>::new());
        // Two cards with the same tags share all of them, each once.
        let twins = [tags(&["a", "b", "a"]), tags(&["b", "a"])];
        assert_eq!(
            ids(&shared_tags(twins.iter().map(Vec::as_slice))),
            ["id-a", "id-b"]
        );
        // Tags are told apart by id, not by name.
        let same_name = [vec![tag("1", "x")], vec![tag("2", "x")]];
        assert_eq!(
            shared_tags(same_name.iter().map(Vec::as_slice)),
            Vec::<Tag>::new()
        );
    }

    /// 200 cards with the same 5,000 tags (an import can make that): the
    /// shared tags and a row's pills take time in the number of tags, not
    /// its square (CPU time, ADR 0049).
    #[test]
    fn thousands_of_tags_on_every_card_cost_little() {
        let tags: Vec<Tag> = (0..5_000)
            .map(|i| tag(&format!("id-{i}"), &format!("t{i}")))
            .collect();
        let names: Vec<String> = tags.iter().map(|t| t.name.clone()).collect();
        let cards = vec![tags.clone(); 200];
        let (cpu, (shared, others)) = crate::testing::cpu_time_of(|| {
            let shared = shared_tags(cards.iter().map(Vec::as_slice));
            let ids = ids(&shared);
            let others: usize = (0..200)
                .map(|_| other_tags(&names, &tags, &ids).len())
                .sum();
            (shared.len(), others)
        });
        assert_eq!((shared, others), (5_000, 0));
        crate::testing::print_timing("shared tags: 200 cards × 5,000 tags (CPU)", cpu);
        // About 0.7 s in a debug build; quadratic in the tags, it would be
        // billions of comparisons.
        crate::testing::assert_cpu_within(
            cpu,
            std::time::Duration::from_secs(5),
            "200 cards × 5,000 tags",
        );
    }

    #[test]
    fn a_row_shows_only_the_tags_not_shared() {
        use crate::widgets::tag_pills::text::{COMPACT_ROW_PILLS, overflow_text, split};

        let card = tags(&["Définition", "Analyse", "MAT3101", "SIC3101"]);
        let names: Vec<String> = card.iter().map(|t| t.name.clone()).collect();
        let shared = ids(&tags(&["Définition", "SIC3101"]));
        let others = other_tags(&names, &card, &shared);
        assert_eq!(*others, ["Analyse", "MAT3101"]);
        // Nothing shared: every tag, borrowed.
        assert!(matches!(other_tags(&names, &card, &[]), Cow::Borrowed(_)));
        // Every tag shared: no pill.
        assert!(other_tags(&names, &card, &ids(&card)).is_empty());
        // Past the pills that fit, the rest is one "+N" pill.
        let (shown, hidden) = split(others.len(), Some(COMPACT_ROW_PILLS));
        assert_eq!((shown, overflow_text(hidden)), (1, "+1".to_owned()));
    }

    #[test]
    fn a_row_offers_only_what_changes_the_card() {
        assert_eq!(
            row_actions(false, false),
            [
                ("row.suspend", true),
                ("row.unsuspend", false),
                ("row.reset", true)
            ]
        );
        assert_eq!(
            row_actions(true, true),
            [
                ("row.suspend", false),
                ("row.unsuspend", true),
                ("row.reset", false)
            ]
        );
    }
}
