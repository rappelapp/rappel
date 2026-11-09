//! What a group header of the deck page says (ADR 0079), pure and
//! unit-tested.

use crate::i18n::{fill, gettext, pgettext};
use crate::pages::browser_text::count_text;

/// The words before a group's tags: "Also" (the cards have these tags
/// besides the shared ones), "Tagged" when no tag is shared (no "On every
/// card:" line above the list), "Only the shared tags" or "No tags" for
/// the cards without any other tag. `extra` is whether the group has tags
/// of its own, `shared` whether the list has shared tags.
pub fn title(extra: bool, shared: bool) -> String {
    match (extra, shared) {
        // Translators: a header in a deck's list of cards, before tag
        // pills: the cards under it have these tags too, besides those
        // on every card.
        (true, true) => pgettext("group header", "Also"),
        // Translators: a header in a deck's list of cards, before tag
        // pills, when no tag is on every card of the deck.
        (true, false) => pgettext("group header", "Tagged"),
        // Translators: a header in a deck's list of cards: the cards
        // under it have no tag but those on every card.
        (false, true) => pgettext("group header", "Only the shared tags"),
        // Translators: a header in a deck's list of cards: the cards
        // under it have no tag.
        (false, false) => pgettext("group header", "No tags"),
    }
}

/// "14 cards", at the end of a group's header.
pub fn count(n: u32) -> String {
    count_text(usize::try_from(n).unwrap_or(usize::MAX))
}

/// What a screen reader says for a group's header: its title, the tags
/// in words (at most 20, then how many more) and the count, "Also tagged
/// Analyse, MAT3101, 7 cards" or "Only the shared tags, 14 cards".
pub fn spoken(tags: &[String], shared: bool, n: u32) -> String {
    let count = count(n);
    if tags.is_empty() {
        return fill(
            // Translators: what a screen reader says for a group of
            // cards: {title} is "Only the shared tags" or "No tags",
            // {count} "14 cards".
            &gettext("{title}, {count}"),
            &[("title", &title(false, shared)), ("count", &count)],
        );
    }
    let names = crate::ui::text::name_list(tags, str::to_owned);
    let template = if shared {
        // Translators: what a screen reader says for a group of cards
        // that have these tags besides the shared ones: {tags} is
        // "Analyse, MAT3101", {count} "7 cards".
        gettext("Also tagged {tags}, {count}")
    } else {
        // Translators: the same, when no tag is on every card.
        gettext("Tagged {tags}, {count}")
    };
    fill(&template, &[("tags", &names), ("count", &count)])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(list: &[&str]) -> Vec<String> {
        list.iter().map(|&s| s.to_owned()).collect()
    }

    #[test]
    fn a_header_names_its_group_and_counts_it() {
        assert_eq!(title(true, true), "Also");
        assert_eq!(title(true, false), "Tagged");
        assert_eq!(title(false, true), "Only the shared tags");
        assert_eq!(title(false, false), "No tags");
        assert_eq!(count(1), "1 card");
        assert_eq!(count(1_234), "1,234 cards");
        assert_eq!(
            spoken(&names(&["Analyse", "MAT3101"]), true, 7),
            "Also tagged Analyse, MAT3101, 7 cards"
        );
        assert_eq!(spoken(&[], true, 14), "Only the shared tags, 14 cards");
        assert_eq!(spoken(&names(&["x"]), false, 2), "Tagged x, 2 cards");
        // Thousands of tags: twenty named, the rest counted.
        let many: Vec<String> = (0..500).map(|i| format!("t{i}")).collect();
        let said = spoken(&many, true, 3);
        assert!(said.contains(" and 480 more"), "{said}");
    }

    #[test]
    fn a_header_is_said_in_french_too() {
        crate::i18n::tests::in_french(|| {
            assert_eq!(title(true, true), "Aussi");
            assert_eq!(title(false, true), "Seulement les étiquettes communes");
            assert_eq!(count(1), "1 carte");
            assert_eq!(count(3), "3 cartes");
            assert_eq!(
                spoken(&names(&["Analyse"]), true, 2),
                "Également\u{a0}: Analyse, 2 cartes"
            );
        });
    }
}
