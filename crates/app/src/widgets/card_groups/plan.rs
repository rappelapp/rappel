//! How a deck page groups its cards (ADR 0079), pure: the tags every card
//! has (said once above the list), each card's other tags (its "extra"
//! tags), the groups of cards with the same extra tags, whether grouping
//! helps, and the order the list shows the cards in.
//!
//! One pass over the cards' tags, a hash of each card's extra tag ids, a
//! sort of the groups only (at most [`MAX_GROUPS`] when they are kept) and
//! a counting sort of the cards: time in the total number of tags, never
//! a sort of the cards themselves.

use std::collections::{HashMap, HashSet};

use rappel_core::domain::{Tag, name_key};

use crate::widgets::card_row::text::SharedTags;

/// The most groups a deck page shows; past it, plain rows.
pub const MAX_GROUPS: usize = 6;

/// One group of cards: those whose extra tags are `tags`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Group {
    /// The extra tags, in the cards' own order (by name key); empty for
    /// the cards with only the shared tags.
    pub tags: Vec<Tag>,
    /// Where the group starts in the list shown.
    pub start: u32,
    /// How many cards it holds (at least one).
    pub len: u32,
}

/// How a list of cards is shown.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Plan {
    /// The tags every card has (none for fewer than two cards).
    pub shared: Vec<Tag>,
    /// The groups, in the order shown; empty when the cards are not
    /// grouped (plain rows in their own order).
    pub groups: Vec<Group>,
    /// The cards' positions in the order shown, when grouped (`order[i]`
    /// is the card shown at `i`); empty otherwise.
    pub order: Vec<u32>,
}

impl Plan {
    /// Whether the cards are shown in groups.
    pub fn grouped(&self) -> bool {
        !self.groups.is_empty()
    }
}

/// Whether groups of `sizes` cards help to read the list: at least two
/// groups (one says nothing the shared tags do not), at most
/// [`MAX_GROUPS`], and not most of them holding a single card (then each
/// header names one card's tags: plain rows with their pills say it in
/// less room). Pure.
pub fn grouping_helps(sizes: &[usize]) -> bool {
    let groups = sizes.len();
    if !(2..=MAX_GROUPS).contains(&groups) {
        return false;
    }
    let single = sizes.iter().filter(|&&n| n == 1).count();
    single * 2 <= groups
}

/// The [`Plan`] of a list whose cards have `cards` tags each (each list
/// in the card's order), in the list's order.
pub fn plan<'a>(cards: impl ExactSizeIterator<Item = &'a [Tag]> + Clone) -> Plan {
    let n = cards.len();
    let mut shared = SharedTags::default();
    for tags in cards.clone() {
        if !shared.add(tags) {
            break;
        }
    }
    let shared = shared.finish();
    let shared_ids: HashSet<&str> = shared.iter().map(|t| t.id.as_str()).collect();

    // Each card's group, by its extra tag ids, in the order first seen.
    let mut index: HashMap<Vec<&str>, usize> = HashMap::new();
    let mut first_of: Vec<&[Tag]> = Vec::new();
    let mut sizes: Vec<usize> = Vec::new();
    let mut group_of: Vec<usize> = Vec::with_capacity(n);
    for tags in cards {
        // Sorted, so the same set is one key whatever order a card's
        // tags come in (each card's own order is by name key, but the
        // key must not rely on it).
        let mut key: Vec<&str> = tags
            .iter()
            .map(|t| t.id.as_str())
            .filter(|id| !shared_ids.contains(id))
            .collect();
        key.sort_unstable();
        key.dedup();
        let next = sizes.len();
        let group = *index.entry(key).or_insert(next);
        if group == next {
            first_of.push(tags);
            sizes.push(0);
        }
        sizes[group] += 1;
        group_of.push(group);
        // Past the cap, the groups are dropped anyway: no need to go on
        // counting them, only to know there are too many.
        if sizes.len() > MAX_GROUPS {
            return Plan {
                shared,
                ..Plan::default()
            };
        }
    }
    if !grouping_helps(&sizes) {
        return Plan {
            shared,
            ..Plan::default()
        };
    }

    let extra = |tags: &[Tag]| -> Vec<Tag> {
        tags.iter()
            .filter(|t| !shared_ids.contains(t.id.as_str()))
            .cloned()
            .collect()
    };
    let mut groups: Vec<(usize, Vec<Tag>, Vec<String>)> = first_of
        .iter()
        .enumerate()
        .map(|(g, tags)| {
            let tags = extra(tags);
            let keys = tags.iter().map(|t| name_key(&t.name)).collect();
            (g, tags, keys)
        })
        .collect();
    // The cards with no extra tag first (an empty key sorts first), then
    // by the names' keys, tag by tag; ids only break a tie no two live
    // tags can make (names are unique by key).
    groups.sort_by(|a, b| {
        a.2.cmp(&b.2)
            .then_with(|| a.1.iter().map(|t| &t.id).cmp(b.1.iter().map(|t| &t.id)))
    });

    // A counting sort: each card goes to its group's next place, so the
    // cards keep their order inside a group.
    let mut rank = vec![0usize; groups.len()];
    let mut next = vec![0u32; groups.len()];
    let mut start = 0u32;
    let mut shown = Vec::with_capacity(groups.len());
    for (r, (g, tags, _)) in groups.into_iter().enumerate() {
        rank[g] = r;
        next[r] = start;
        let len = u32::try_from(sizes[g]).unwrap_or(u32::MAX);
        shown.push(Group { tags, start, len });
        start = start.saturating_add(len);
    }
    let mut order = vec![0u32; n];
    for (position, &g) in group_of.iter().enumerate() {
        let r = rank[g];
        if let (Some(slot), Ok(position)) = (
            usize::try_from(next[r]).ok().and_then(|i| order.get_mut(i)),
            u32::try_from(position),
        ) {
            *slot = position;
        }
        next[r] = next[r].saturating_add(1);
    }
    Plan {
        shared,
        groups: shown,
        order,
    }
}

/// The group holding position `position` of the list shown, by its start
/// (the groups in order, as in [`Plan::groups`]); `None` past the end.
pub fn group_at(groups: &[Group], position: u32) -> Option<&Group> {
    let i = groups.partition_point(|g| g.start <= position);
    let group = groups.get(i.checked_sub(1)?)?;
    (position < group.start.saturating_add(group.len)).then_some(group)
}

/// Whether two plans' headers show the same: the same groups (starts,
/// sizes and tags) and the same answer to "is any tag shared", which
/// chooses between "Also" and "Tagged", "Only the shared tags" and "No
/// tags".
pub fn same_headers(a: &Plan, b: &Plan) -> bool {
    a.shared.is_empty() == b.shared.is_empty()
        && a.groups.len() == b.groups.len()
        && a.groups.iter().zip(&b.groups).all(|(a, b)| {
            a.start == b.start
                && a.len == b.len
                && a.tags.len() == b.tags.len()
                && a.tags
                    .iter()
                    .zip(&b.tags)
                    .all(|(x, y)| x.id == y.id && x.name == y.name)
        })
}

/// The first and last places where two lists differ, as one
/// `items-changed`: the position, how many left and how many came.
/// `None` when they are the same. `same` compares two items. Pure.
pub fn changed_span<T>(
    old: &[T],
    new: &[T],
    same: impl Fn(&T, &T) -> bool,
) -> Option<(usize, usize, usize)> {
    let prefix = old.iter().zip(new).take_while(|(a, b)| same(a, b)).count();
    if prefix == old.len() && prefix == new.len() {
        return None;
    }
    let room = old.len().min(new.len()) - prefix;
    let suffix = old
        .iter()
        .rev()
        .zip(new.iter().rev())
        .take(room)
        .take_while(|(a, b)| same(a, b))
        .count();
    Some((
        prefix,
        old.len() - prefix - suffix,
        new.len() - prefix - suffix,
    ))
}

/// Orders tags as a group header lists them, for a test's expectations.
#[cfg(test)]
fn by_key(a: &str, b: &str) -> std::cmp::Ordering {
    name_key(a).cmp(&name_key(b))
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;

    fn tag(name: &str) -> Tag {
        Tag {
            id: format!("id-{name}"),
            name: name.to_owned(),
        }
    }

    fn tags(names: &[&str]) -> Vec<Tag> {
        let mut names = names.to_vec();
        names.sort_by(|a, b| by_key(a, b));
        names.into_iter().map(tag).collect()
    }

    fn plan_of(cards: &[Vec<Tag>]) -> Plan {
        plan(cards.iter().map(Vec::as_slice))
    }

    fn names(group: &Group) -> Vec<&str> {
        group.tags.iter().map(|t| t.name.as_str()).collect()
    }

    #[test]
    fn grouping_helps_with_a_few_groups_of_several_cards() {
        // One group (or none) says nothing.
        assert!(!grouping_helps(&[]));
        assert!(!grouping_helps(&[12]));
        // Two to six groups.
        assert!(grouping_helps(&[14, 7]));
        assert!(grouping_helps(&[3, 3, 3, 3, 3, 3]));
        assert!(!grouping_helps(&[3, 3, 3, 3, 3, 3, 3]));
        // Half the groups with one card is still fine; more is not.
        assert!(grouping_helps(&[20, 1]));
        assert!(grouping_helps(&[5, 1, 4, 1]));
        assert!(!grouping_helps(&[5, 1, 1]));
        assert!(!grouping_helps(&[1, 1, 1, 1, 9]));
    }

    /// The mockup's deck: 21 cards sharing three tags, 7 of them with two
    /// more. The cards with only the shared tags come first, each group
    /// keeps the deck's order.
    #[test]
    fn the_mockups_deck_has_two_groups_in_the_decks_order() {
        let shared = ["Mathématiques", "Probabilités", "SIC3101"];
        let extra = [7, 8, 10, 11, 12, 17, 18];
        let cards: Vec<Vec<Tag>> = (0..21)
            .map(|i| {
                let mut list: Vec<&str> = shared.to_vec();
                if extra.contains(&i) {
                    list.extend(["Analyse", "MAT3101"]);
                }
                tags(&list)
            })
            .collect();
        let plan = plan_of(&cards);
        assert_eq!(
            plan.shared
                .iter()
                .map(|t| t.name.as_str())
                .collect::<Vec<_>>(),
            shared
        );
        assert!(plan.grouped());
        assert_eq!(plan.groups.len(), 2);
        assert_eq!(plan.groups[0].tags, []);
        assert_eq!((plan.groups[0].start, plan.groups[0].len), (0, 14));
        assert_eq!(names(&plan.groups[1]), ["Analyse", "MAT3101"]);
        assert_eq!((plan.groups[1].start, plan.groups[1].len), (14, 7));
        let expected: Vec<u32> = (0..21)
            .filter(|i| !extra.contains(i))
            .chain(extra)
            .collect();
        assert_eq!(plan.order, expected);
        assert_eq!(group_at(&plan.groups, 0), Some(&plan.groups[0]));
        assert_eq!(group_at(&plan.groups, 13), Some(&plan.groups[0]));
        assert_eq!(group_at(&plan.groups, 14), Some(&plan.groups[1]));
        assert_eq!(group_at(&plan.groups, 20), Some(&plan.groups[1]));
        assert_eq!(group_at(&plan.groups, 21), None);
    }

    /// Groups other than the first are sorted by their tags' name keys
    /// (case and accents aside), tag by tag.
    #[test]
    fn the_other_groups_are_sorted_by_name_key() {
        let cards = vec![
            tags(&["zèbre"]),
            tags(&["Été", "b"]),
            tags(&["été", "b"]),
            tags(&["a"]),
            tags(&[]),
            tags(&["zèbre"]),
            tags(&["a"]),
            tags(&[]),
            tags(&["Été", "b"]),
        ];
        // "Été" and "été" are two tags here (ids differ), as two decks'
        // imports could not make them, but the order must still be total.
        let plan = plan_of(&cards);
        assert!(plan.grouped(), "{plan:?}");
        let order: Vec<Vec<&str>> = plan.groups.iter().map(names).collect();
        assert_eq!(order[0], Vec::<&str>::new());
        assert_eq!(order[1], ["a"]);
        assert_eq!(order[2], ["b", "Été"]);
        assert_eq!(order[3], ["b", "été"]);
        assert_eq!(order[4], ["zèbre"]);
        assert_eq!(plan.order, [4, 7, 3, 6, 1, 8, 2, 0, 5]);
    }

    /// Without the empty group the first is the first by name.
    #[test]
    fn a_deck_where_every_card_has_an_extra_tag_starts_with_the_first_name() {
        let cards = vec![tags(&["b"]), tags(&["a"]), tags(&["b"]), tags(&["a"])];
        let plan = plan_of(&cards);
        let order: Vec<Vec<&str>> = plan.groups.iter().map(names).collect();
        assert_eq!(order, [["a"], ["b"]]);
        assert_eq!(plan.order, [1, 3, 0, 2]);
    }

    #[test]
    fn too_many_or_too_small_groups_are_plain_rows() {
        // Seven tag sets.
        let cards: Vec<Vec<Tag>> = (0..70)
            .map(|i| tags(&[["a", "b", "c", "d", "e", "f", "g"][i % 7]]))
            .collect();
        let plan = plan_of(&cards);
        assert!(!plan.grouped());
        assert_eq!(plan.order, Vec::<u32>::new());
        // Most groups of one card.
        let cards = vec![tags(&[]), tags(&[]), tags(&["a"]), tags(&["b"])];
        assert!(!plan_of(&cards).grouped());
        // Every card alike, or one card: nothing to group.
        assert!(!plan_of(&[tags(&["a"]), tags(&["a"])]).grouped());
        assert!(!plan_of(&[tags(&["a"])]).grouped());
        assert_eq!(plan_of(&[]), Plan::default());
    }

    /// The same set in another order is one group.
    #[test]
    fn a_set_is_one_group_whatever_its_order() {
        let (a, b, s) = (tag("a"), tag("b"), tag("s"));
        let cards = vec![
            vec![s.clone(), a.clone(), b.clone()],
            vec![s.clone(), b.clone(), a.clone()],
            vec![s.clone()],
            vec![s.clone()],
        ];
        let plan = plan_of(&cards);
        assert_eq!(plan.groups.len(), 2, "{plan:?}");
        assert_eq!(plan.groups[1].len, 2);
    }

    #[test]
    fn headers_change_with_whether_a_tag_is_shared() {
        let shared = plan_of(&[
            tags(&["s"]),
            tags(&["s"]),
            tags(&["s", "x"]),
            tags(&["s", "x"]),
        ]);
        let mut none = shared.clone();
        none.shared.clear();
        assert!(same_headers(&shared, &shared));
        assert!(!same_headers(&shared, &none));
        let mut moved = shared.clone();
        moved.groups[0].len = 1;
        assert!(!same_headers(&shared, &moved));
    }

    /// 10,000 cards with 10,000 different sets: the walk stops past the
    /// cap, nothing is grouped, quickly.
    #[test]
    fn ten_thousand_tag_sets_stop_early() {
        let cards: Vec<Vec<Tag>> = (0..10_000)
            .map(|i| vec![tag("shared"), tag(&format!("t{i:05}"))])
            .collect();
        let (cpu, plan) = crate::testing::cpu_time_of(|| plan_of(&cards));
        assert!(!plan.grouped());
        assert_eq!(plan.shared.len(), 1);
        crate::testing::print_timing("card_groups::plan, 10,000 sets (CPU)", cpu);
        crate::testing::assert_cpu_within(cpu, Duration::from_millis(500), "10,000 sets");
    }

    /// 2,000 cards with 2,000 extra tags each, in two groups: grouped,
    /// the second group's header has all 2,000 names, in time linear in
    /// the 4 million tags.
    #[test]
    fn thousands_of_extra_tags_in_two_groups_are_grouped() {
        let many: Vec<Tag> = (0..2_000).map(|t| tag(&format!("x{t:04}"))).collect();
        let cards: Vec<Vec<Tag>> = (0..2_000)
            .map(|i| if i % 2 == 0 { many.clone() } else { Vec::new() })
            .collect();
        let (cpu, plan) = crate::testing::cpu_time_of(|| plan_of(&cards));
        assert!(plan.grouped());
        assert_eq!(plan.groups.len(), 2);
        assert_eq!(plan.groups[0].tags, []);
        assert_eq!(plan.groups[1].tags.len(), 2_000);
        assert_eq!(plan.groups[1].tags[0].name, "x0000");
        assert_eq!((plan.groups[1].start, plan.groups[1].len), (1_000, 1_000));
        crate::testing::print_timing("card_groups::plan, 2,000 × 2,000 tags (CPU)", cpu);
        crate::testing::assert_cpu_within(cpu, Duration::from_secs(5), "2,000 × 2,000 tags");
    }

    #[test]
    fn a_change_is_one_span() {
        let same = |a: &u32, b: &u32| a == b;
        assert_eq!(changed_span(&[1, 2, 3], &[1, 2, 3], same), None);
        assert_eq!(changed_span::<u32>(&[], &[], same), None);
        assert_eq!(changed_span(&[1, 2, 3], &[1, 9, 3], same), Some((1, 1, 1)));
        assert_eq!(changed_span(&[1, 2, 3], &[1, 3], same), Some((1, 1, 0)));
        assert_eq!(changed_span(&[1, 3], &[1, 2, 3], same), Some((1, 0, 1)));
        assert_eq!(changed_span(&[], &[1, 2], same), Some((0, 0, 2)));
        assert_eq!(changed_span(&[1, 2], &[], same), Some((0, 2, 0)));
        // A card moving from one group to another: the span between.
        assert_eq!(
            changed_span(&[1, 2, 3, 4, 5], &[1, 3, 4, 2, 5], same),
            Some((1, 3, 3))
        );
        // Repeats never overlap prefix and suffix.
        assert_eq!(changed_span(&[1, 1], &[1, 1, 1], same), Some((2, 0, 1)));
    }

    /// 10,000 cards, each with four shared tags and one of five extra
    /// sets: the plan is linear in the tags, measured as CPU time.
    #[test]
    fn ten_thousand_cards_are_grouped_quickly() {
        let sets = ["a", "b", "c", "d", "e"];
        let cards: Vec<Vec<Tag>> = (0..10_000)
            .map(|i| tags(&["w", "x", "y", "z", sets[i % 5]]))
            .collect();
        let (cpu, plan) = crate::testing::cpu_time_of(|| plan_of(&cards));
        assert_eq!(plan.groups.len(), 5);
        assert_eq!(plan.order.len(), 10_000);
        crate::testing::print_timing("card_groups::plan, 10,000 cards (CPU)", cpu);
        crate::testing::assert_cpu_within(cpu, Duration::from_millis(500), "grouping cards");
    }
}
