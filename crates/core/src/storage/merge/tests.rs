use std::time::Duration;

use super::*;
use crate::domain::Grade;
use crate::storage::Scope;
use crate::test_support::{DAY_MS, T0, answer, dump, schedule_table};

fn deleted_at(store: &Store, table: &str, id: &str) -> Option<i64> {
    store
        .conn
        .query_row(
            &format!("SELECT deleted_at FROM {table} WHERE id = ?1"),
            [id],
            |r| r.get(0),
        )
        .unwrap()
}

fn deck_of(store: &Store, card: &str) -> String {
    store
        .conn
        .query_row("SELECT deck_id FROM cards WHERE id = ?1", [card], |r| {
            r.get(0)
        })
        .unwrap()
}

fn live_ids(store: &Store, deck: &str) -> Vec<String> {
    store
        .list_cards(deck)
        .unwrap()
        .into_iter()
        .map(|c| c.id)
        .collect()
}

/// Two decks: "Spanish" with a reviewed and tagged card, a suspended card
/// and a card deleted on its own, and "Languages" with one card.
struct Two {
    store: Store,
    spanish: String,
    languages: String,
    reviewed: String,
    suspended: String,
    deleted: String,
    other: String,
}

fn two() -> Two {
    let mut store = Store::in_memory();
    let spanish = store.create_deck_at("Spanish", T0).unwrap().id;
    let languages = store.create_deck_at("Languages", T0).unwrap().id;
    let reviewed = store
        .add_card_with_tags_at(&spanish, "hola", "hello", &["verbs".to_owned()], T0)
        .unwrap()
        .card
        .id;
    let suspended = store.add_card_at(&spanish, "adiós", "bye", T0).unwrap().id;
    let deleted = store.add_card_at(&spanish, "gato", "cat", T0).unwrap().id;
    let other = store.add_card_at(&languages, "chat", "cat", T0).unwrap().id;
    answer(&mut store, &reviewed, Grade::Pass, T0 + 1);
    answer(&mut store, &deleted, Grade::Fail, T0 + 2);
    store
        .set_suspended(std::slice::from_ref(&suspended), true)
        .unwrap();
    store.delete_card_at(&deleted, T0 + 10).unwrap();
    Two {
        store,
        spanish,
        languages,
        reviewed,
        suspended,
        deleted,
        other,
    }
}

// --- Merge ------------------------------------------------------------------

#[test]
fn merge_moves_every_card_with_its_history_and_deletes_the_source() {
    let Two {
        mut store,
        spanish,
        languages,
        reviewed,
        suspended,
        deleted,
        other,
    } = two();
    let before_cards: Vec<_> = [&reviewed, &suspended]
        .iter()
        .map(|id| store.card(id).unwrap())
        .collect();
    let schedules = schedule_table(&store);
    let tags = store.card_tags(&reviewed).unwrap();
    let log = store.review_events(&deleted).unwrap();

    let merged = store.merge_deck_at(&spanish, &languages, T0 + 20).unwrap();

    assert_eq!(
        merged,
        Merged {
            source: spanish.clone(),
            target: languages.clone(),
            stamp: T0 + 20,
            cards: vec![reviewed.clone(), suspended.clone(), deleted.clone()],
            live: 2,
        }
    );
    // Every card is in the target, the deleted one still deleted with its
    // own stamp.
    for card in [&reviewed, &suspended, &deleted, &other] {
        assert_eq!(deck_of(&store, card), languages);
    }
    assert_eq!(deleted_at(&store, "cards", &deleted), Some(T0 + 10));
    assert_eq!(
        live_ids(&store, &languages),
        [reviewed.clone(), suspended.clone(), other]
    );
    // Content, updated_at, suspension, tags, the log and the schedule stay.
    for card in before_cards {
        let now = store.card(&card.id).unwrap();
        assert_eq!(
            (now.deck_id, now.suspended),
            (languages.clone(), card.suspended)
        );
        assert_eq!(
            (now.front_md, now.back_md, now.created_at, now.updated_at),
            (
                card.front_md,
                card.back_md,
                card.created_at,
                card.updated_at
            )
        );
    }
    assert!(store.card(&suspended).unwrap().suspended);
    assert_eq!(schedule_table(&store), schedules);
    assert_eq!(store.card_tags(&reviewed).unwrap(), tags);
    assert_eq!(store.review_events(&deleted).unwrap(), log);
    // The source is deleted, as Delete Deck would leave it.
    assert_eq!(deleted_at(&store, "decks", &spanish), Some(T0 + 20));
    assert!(matches!(store.deck(&spanish), Err(Error::DeckNotFound(_))));
    assert_eq!(store.decks().unwrap().len(), 1);
}

#[test]
fn merging_an_empty_deck_only_deletes_it() {
    let mut store = Store::in_memory();
    let empty = store.create_deck_at("Empty", T0).unwrap().id;
    let target = store.create_deck_at("Target", T0).unwrap().id;
    let merged = store.merge_deck_at(&empty, &target, T0 + 1).unwrap();
    assert_eq!((merged.cards.len(), merged.live), (0, 0));
    assert_eq!(deleted_at(&store, "decks", &empty), Some(T0 + 1));
}

#[test]
fn a_deck_cannot_be_merged_into_itself() {
    let Two {
        mut store, spanish, ..
    } = two();
    let before = dump(&store);
    let err = store
        .merge_deck_at(&spanish, &spanish, T0 + 20)
        .unwrap_err();
    assert!(
        matches!(&err, Error::MergeIntoItself(id) if *id == spanish),
        "{err:?}"
    );
    assert_eq!(dump(&store), before);
}

#[test]
fn missing_or_deleted_decks_cannot_be_merged() {
    let Two {
        mut store,
        spanish,
        languages,
        ..
    } = two();
    let gone = store.create_deck_at("Gone", T0).unwrap().id;
    store.delete_deck_at(&gone, T0 + 5).unwrap();
    let before = dump(&store);
    for (source, target, missing) in [
        (&gone, &languages, &gone),
        (&spanish, &gone, &gone),
        (&"nope".to_owned(), &languages, &"nope".to_owned()),
        (&spanish, &"nope".to_owned(), &"nope".to_owned()),
    ] {
        let err = store.merge_deck_at(source, target, T0 + 20).unwrap_err();
        assert!(
            matches!(&err, Error::DeckNotFound(id) if id == missing),
            "{err:?}"
        );
        assert_eq!(dump(&store), before);
    }
}

#[test]
fn the_source_stamp_is_after_its_cards_deleted_earlier() {
    // A clock that went backwards: the card was deleted "later" than the
    // merge, so the deck's stamp is one past it, as Delete Deck makes it.
    let Two {
        mut store,
        spanish,
        languages,
        ..
    } = two();
    let merged = store.merge_deck_at(&spanish, &languages, T0 + 5).unwrap();
    assert_eq!(merged.stamp, T0 + 11);
    assert_eq!(deleted_at(&store, "decks", &spanish), Some(T0 + 11));
}

/// A failure at the last write, the source's delete, after every card
/// moved, leaves the database as it was: the merge is one transaction.
/// The failure is a temporary trigger, as in the review tests.
#[test]
fn a_failure_at_the_last_write_writes_nothing() {
    let Two {
        mut store,
        spanish,
        languages,
        ..
    } = two();
    let before = dump(&store);
    store
        .conn
        .execute_batch(
            "CREATE TEMP TRIGGER fail_merge BEFORE UPDATE OF deleted_at ON decks
             BEGIN SELECT RAISE(ABORT, 'merge failed'); END;",
        )
        .unwrap();
    let err = store
        .merge_deck_at(&spanish, &languages, T0 + 20)
        .unwrap_err();
    assert!(err.to_string().contains("merge failed"), "{err:?}");
    store
        .conn
        .execute_batch("DROP TRIGGER temp.fail_merge;")
        .unwrap();
    assert_eq!(dump(&store), before);
}

/// The case the stamps are for: a card deleted before the merge, then the
/// target deleted and its delete undone. Only the target's own batch comes
/// back, even with a clock that went backwards in between.
#[test]
fn undoing_a_later_delete_of_the_target_does_not_revive_cards_deleted_before_the_merge() {
    let Two {
        mut store,
        spanish,
        languages,
        reviewed,
        suspended,
        deleted,
        other,
    } = two();
    store.merge_deck_at(&spanish, &languages, T0 + 20).unwrap();
    for at in [T0 + 30, T0 + 3] {
        store.delete_deck_at(&languages, at).unwrap();
        let stamp = deleted_at(&store, "decks", &languages).unwrap();
        assert!(stamp > T0 + 10, "past the earlier delete: {stamp}");
        store.undelete_deck(&languages).unwrap();
        assert_eq!(deleted_at(&store, "cards", &deleted), Some(T0 + 10));
        assert_eq!(
            live_ids(&store, &languages),
            [reviewed.clone(), suspended.clone(), other.clone()]
        );
    }
}

/// Reviving the merged deck (an import of a file where it is live does
/// this, SPEC §4.1) brings it back empty: no card has its id any more.
#[test]
fn reviving_the_merged_deck_brings_back_no_card() {
    let Two {
        mut store,
        spanish,
        languages,
        deleted,
        ..
    } = two();
    store.merge_deck_at(&spanish, &languages, T0 + 20).unwrap();
    store.undelete_deck(&spanish).unwrap();
    assert_eq!(live_ids(&store, &spanish), Vec::<String>::new());
    assert_eq!(live_ids(&store, &languages).len(), 3);
    assert_eq!(deleted_at(&store, "cards", &deleted), Some(T0 + 10));
}

#[test]
fn card_count_counts_the_live_cards_suspended_ones_included() {
    let Two {
        mut store,
        spanish,
        languages,
        ..
    } = two();
    assert_eq!(store.card_count(&spanish).unwrap(), 2);
    assert_eq!(store.card_count(&languages).unwrap(), 1);
    store.merge_deck_at(&spanish, &languages, T0 + 20).unwrap();
    assert_eq!(store.card_count(&languages).unwrap(), 3);
    assert!(matches!(
        store.card_count(&spanish),
        Err(Error::DeckNotFound(_))
    ));
}

/// After a merge the target has to study what both had, and the
/// statistics of every deck are the same as before.
#[test]
fn study_counts_and_statistics_after_a_merge_are_the_sums() {
    use crate::stats::DayGrid;
    let Two {
        mut store,
        spanish,
        languages,
        other,
        ..
    } = two();
    answer(&mut store, &other, Grade::Pass, T0 + 3);
    store.set_max_new_per_day(9_999).unwrap();
    store.set_max_reviews_per_day(9_999).unwrap();
    let now = T0 + 30 * DAY_MS;
    let past = DayGrid::new((0..=40).map(|i| now - (40 - i) * DAY_MS).collect()).unwrap();
    let ahead = DayGrid::new((0..=30).map(|i| now + i * DAY_MS).collect()).unwrap();
    let all = store.stats(Scope::All, now, &past, &ahead).unwrap();
    let a = store
        .stats(Scope::Deck(&spanish), now, &past, &ahead)
        .unwrap();
    let b = store
        .stats(Scope::Deck(&languages), now, &past, &ahead)
        .unwrap();
    let decks = store.list_decks(now, now).unwrap();

    store.merge_deck_at(&spanish, &languages, T0 + 20).unwrap();

    let after = store.list_decks(now, now).unwrap();
    assert_eq!(after.len(), 1);
    let sum = |f: fn(&crate::domain::DeckSummary) -> u32| decks.iter().map(f).sum::<u32>();
    let merged = &after[0];
    assert_eq!(merged.new_count, sum(|d| d.new_count));
    assert_eq!(merged.due_count, sum(|d| d.due_count));
    assert_eq!(merged.card_count, sum(|d| d.card_count));
    assert_eq!(merged.suspended_count, sum(|d| d.suspended_count));
    assert_eq!(merged.held_back_count, sum(|d| d.held_back_count));
    assert_eq!(
        merged.next_change_at,
        decks.iter().filter_map(|d| d.next_change_at).min()
    );
    let m = store
        .stats(Scope::Deck(&languages), now, &past, &ahead)
        .unwrap();
    assert_eq!(store.stats(Scope::All, now, &past, &ahead).unwrap(), all);
    assert_eq!(m.scheduled_cards, a.scheduled_cards + b.scheduled_cards);
    assert_eq!(m.overdue, a.overdue + b.overdue);
    for (i, day) in m.forecast.iter().enumerate() {
        assert_eq!(*day, a.forecast[i] + b.forecast[i]);
    }
    for (i, day) in m.days.iter().enumerate() {
        assert_eq!(day.reviews, a.days[i].reviews + b.days[i].reviews);
        assert_eq!(day.passes, a.days[i].passes + b.days[i].passes);
    }
}

// --- Unmerge ----------------------------------------------------------------

#[test]
fn unmerge_restores_the_database_exactly() {
    let Two {
        mut store,
        spanish,
        languages,
        ..
    } = two();
    let before = dump(&store);
    let merged = store.merge_deck_at(&spanish, &languages, T0 + 20).unwrap();
    assert_ne!(dump(&store), before, "the dump sees the merge");
    let deck = store.unmerge_deck(&merged).unwrap();
    assert_eq!(deck, store.deck(&spanish).unwrap());
    assert_eq!(deck.name, "Spanish");
    assert_eq!(dump(&store), before);
}

/// What happened to a card since the merge: deleted or edited or studied,
/// it goes back (the undo is about where cards are, not what they say);
/// moved to a third deck, it stays there, as after Move to Deck's Undo.
#[test]
fn unmerge_takes_back_the_cards_still_in_the_target() {
    let Two {
        mut store,
        spanish,
        languages,
        reviewed,
        suspended,
        deleted,
        other,
    } = two();
    let third = store.create_deck_at("Third", T0).unwrap().id;
    let fourth = store.add_card_at(&spanish, "perro", "dog", T0).unwrap().id;
    let merged = store.merge_deck_at(&spanish, &languages, T0 + 20).unwrap();
    store
        .edit_card_at(&reviewed, "¡hola!", "hello", T0 + 21)
        .unwrap();
    answer(&mut store, &reviewed, Grade::Pass, T0 + DAY_MS);
    store.delete_card_at(&suspended, T0 + 22).unwrap();
    store
        .move_cards(std::slice::from_ref(&fourth), &third)
        .unwrap();

    store.unmerge_deck(&merged).unwrap();

    assert_eq!(deck_of(&store, &reviewed), spanish);
    assert_eq!(store.card(&reviewed).unwrap().front_md, "¡hola!");
    assert_eq!(store.review_events(&reviewed).unwrap().len(), 2);
    assert_eq!(deck_of(&store, &suspended), spanish);
    assert_eq!(deleted_at(&store, "cards", &suspended), Some(T0 + 22));
    assert_eq!(deck_of(&store, &deleted), spanish);
    assert_eq!(deck_of(&store, &fourth), third);
    // The target's own card never moves.
    assert_eq!(deck_of(&store, &other), languages);
    assert_eq!(live_ids(&store, &spanish), [reviewed]);
    // The card deleted since can still be brought back, into the source.
    assert_eq!(
        store
            .undelete_cards(std::slice::from_ref(&suspended))
            .unwrap(),
        1
    );
    assert_eq!(deck_of(&store, &suspended), spanish);
}

#[test]
fn unmerge_refuses_a_name_taken_meanwhile_and_changes_nothing() {
    let Two {
        mut store,
        spanish,
        languages,
        ..
    } = two();
    let merged = store.merge_deck_at(&spanish, &languages, T0 + 20).unwrap();
    store.create_deck_at("SPANISH", T0 + 21).unwrap();
    let before = dump(&store);
    let err = store.unmerge_deck(&merged).unwrap_err();
    assert!(
        matches!(&err, Error::DuplicateDeckName(n) if n == "Spanish"),
        "{err:?}"
    );
    assert_eq!(dump(&store), before);
}

#[test]
fn unmerge_refuses_a_deleted_target_and_changes_nothing() {
    let Two {
        mut store,
        spanish,
        languages,
        ..
    } = two();
    let merged = store.merge_deck_at(&spanish, &languages, T0 + 20).unwrap();
    store.delete_deck_at(&languages, T0 + 21).unwrap();
    let before = dump(&store);
    let err = store.unmerge_deck(&merged).unwrap_err();
    assert!(
        matches!(&err, Error::DeckNotFound(id) if *id == languages),
        "{err:?}"
    );
    assert_eq!(dump(&store), before);
    // Once the target's delete is undone, the merge can be.
    let before_merge_cards = 2;
    store.undelete_deck(&languages).unwrap();
    store.unmerge_deck(&merged).unwrap();
    assert_eq!(live_ids(&store, &spanish).len(), before_merge_cards);
}

#[test]
fn unmerge_of_a_source_revived_meanwhile_gives_it_its_cards_back() {
    let Two {
        mut store,
        spanish,
        languages,
        ..
    } = two();
    let before = dump(&store);
    let merged = store.merge_deck_at(&spanish, &languages, T0 + 20).unwrap();
    store.undelete_deck(&spanish).unwrap();
    store.unmerge_deck(&merged).unwrap();
    assert_eq!(dump(&store), before);
    // A second undo finds nothing left to do.
    store.unmerge_deck(&merged).unwrap();
    assert_eq!(dump(&store), before);
}

#[test]
fn unmerge_of_a_source_deleted_again_since_is_refused() {
    let Two {
        mut store,
        spanish,
        languages,
        ..
    } = two();
    let merged = store.merge_deck_at(&spanish, &languages, T0 + 20).unwrap();
    store.undelete_deck(&spanish).unwrap();
    store.delete_deck_at(&spanish, T0 + 30).unwrap();
    let before = dump(&store);
    let err = store.unmerge_deck(&merged).unwrap_err();
    assert!(
        matches!(&err, Error::DeckNotFound(id) if *id == spanish),
        "{err:?}"
    );
    assert_eq!(dump(&store), before);
    let missing = Merged {
        source: "nope".to_owned(),
        ..merged
    };
    assert!(matches!(
        store.unmerge_deck(&missing),
        Err(Error::DeckNotFound(id)) if id == "nope"
    ));
    assert_eq!(dump(&store), before);
}

/// Card ids that would need escaping in JSON go back like any other (the
/// ids are passed to SQLite as a JSON array).
#[test]
fn unmerge_reads_any_id_back() {
    let Two {
        mut store,
        spanish,
        languages,
        ..
    } = two();
    let merged = store.merge_deck_at(&spanish, &languages, T0 + 20).unwrap();
    let odd = Merged {
        cards: vec!["\"]".to_owned(), "\\".to_owned(), "é\u{1}".to_owned()],
        ..merged.clone()
    };
    // None of those is a card: the deck comes back, no card does.
    store.unmerge_deck(&odd).unwrap();
    assert_eq!(live_ids(&store, &spanish), Vec::<String>::new());
    // The real list still works after it.
    let back = store.unmerge_deck(&merged);
    assert!(back.is_ok());
    assert_eq!(live_ids(&store, &spanish).len(), 2);
}

// --- Merge then unmerge is the identity, on any database --------------------

mod identity {
    use proptest::prelude::*;

    use super::*;

    const DECKS: usize = 4;
    const CARDS: usize = 14;

    /// One random write on a card: an answer, a suspension, an undo of the
    /// card's latest entry, a reset, a tag, a move or a deletion.
    #[derive(Debug, Clone, Copy)]
    enum Op {
        Pass,
        Fail,
        Suspend,
        Unsuspend,
        UndoLatest,
        Reset,
        Tag,
        Move(usize),
        Delete,
    }

    fn op() -> impl Strategy<Value = Op> {
        prop_oneof![
            3 => Just(Op::Pass),
            2 => Just(Op::Fail),
            1 => Just(Op::Suspend),
            1 => Just(Op::Unsuspend),
            1 => Just(Op::UndoLatest),
            1 => Just(Op::Reset),
            1 => Just(Op::Tag),
            1 => (0..DECKS).prop_map(Op::Move),
            1 => Just(Op::Delete),
        ]
    }

    proptest! {
        // About 0.1 s per 16 cases in a debug build.
        #![proptest_config(ProptestConfig::with_cases(128))]

        /// Merging any live deck into any other, then undoing it, leaves
        /// every row of every table as it was; in between, the target has
        /// to study the sum of what both had.
        #[test]
        fn merge_then_unmerge_is_the_identity(
            homes in prop::collection::vec(0..DECKS, CARDS),
            ops in prop::collection::vec((op(), 0..CARDS, -60_i64..3_000), 0..60),
            delete_deck in prop::option::of(0..DECKS),
            pair in (0..DECKS, 0..DECKS),
            probe in 0_i64..40_000,
            // The merge's clock, from a day before the last write (a clock
            // that went backwards) to a day after.
            offset in -DAY_MS..DAY_MS,
        ) {
            let mut store = Store::in_memory();
            store.set_max_new_per_day(9_999).unwrap();
            store.set_max_reviews_per_day(9_999).unwrap();
            let decks: Vec<String> = (0..DECKS)
                .map(|i| store.create_deck_at(&format!("Deck {i}"), T0).unwrap().id)
                .collect();
            let cards: Vec<String> = homes
                .iter()
                .enumerate()
                .map(|(i, &d)| {
                    store.add_card_at(&decks[d], &format!("q{i}"), "a", T0).unwrap().id
                })
                .collect();
            let mut latest: Vec<Option<String>> = vec![None; CARDS];
            let mut t = T0;
            for (op, i, minutes) in ops {
                t += minutes * 60_000;
                let card = &cards[i];
                let one = std::slice::from_ref(card);
                // Writes on a deleted card fail; that is part of the
                // history too.
                match op {
                    Op::Pass | Op::Fail => {
                        let grade = if matches!(op, Op::Pass) { Grade::Pass } else { Grade::Fail };
                        if let Ok(e) = store.record_review(card, grade, Duration::from_secs(1), t) {
                            latest[i] = Some(e.id);
                        }
                    }
                    Op::Suspend => { let _ = store.set_suspended(one, true); }
                    Op::Unsuspend => { let _ = store.set_suspended(one, false); }
                    Op::UndoLatest => {
                        if let Some(e) = latest[i].take() {
                            let _ = store.undo(card, &e, t);
                        }
                    }
                    Op::Reset => {
                        if let Ok(e) = store.reset_card(card, t) {
                            latest[i] = Some(e.id);
                        }
                    }
                    Op::Tag => {
                        let _ = store.add_tags_to_cards_at(one, &[format!("t{}", i % 3)], 100, t);
                    }
                    Op::Move(d) => { let _ = store.move_cards(one, &decks[d]); }
                    Op::Delete => { let _ = store.delete_card_at(card, t); }
                }
            }
            // An earlier deleted deck, whose cards are deleted with it.
            if let Some(d) = delete_deck {
                t += 1;
                store.delete_deck_at(&decks[d], t).unwrap();
            }
            let live: Vec<&String> = decks
                .iter()
                .enumerate()
                .filter(|&(i, _)| Some(i) != delete_deck)
                .map(|(_, d)| d)
                .collect();
            let (source, target) = (live[pair.0 % live.len()], live[pair.1 % live.len()]);
            prop_assume!(source != target);
            let before = dump(&store);
            let source_cards: Vec<String> = store
                .conn
                .prepare("SELECT id FROM cards WHERE deck_id = ?1 ORDER BY seq")
                .unwrap()
                .query_map([source], |r| r.get(0))
                .unwrap()
                .collect::<rusqlite::Result<_>>()
                .unwrap();
            let now = T0 + probe * 60_000;
            let counts = store.list_decks(now, now).unwrap();

            let merged = store.merge_deck_at(source, target, t + offset).unwrap();

            // Every card of the source, live and deleted, moved, and no
            // card is left in it.
            prop_assert_eq!(&merged.cards, &source_cards);
            let left: i64 = store
                .conn
                .query_row("SELECT count(*) FROM cards WHERE deck_id = ?1", [source], |r| r.get(0))
                .unwrap();
            prop_assert_eq!(left, 0);

            // The target has what both had.
            let after = store.list_decks(now, now).unwrap();
            let of = |list: &[crate::domain::DeckSummary], id: &str| {
                list.iter().find(|d| d.deck.id == id).cloned()
            };
            let (s, d, m) = (
                of(&counts, source).unwrap(),
                of(&counts, target).unwrap(),
                of(&after, target).unwrap(),
            );
            prop_assert!(of(&after, source).is_none());
            prop_assert_eq!(m.new_count, s.new_count + d.new_count);
            prop_assert_eq!(m.due_count, s.due_count + d.due_count);
            prop_assert_eq!(m.card_count, s.card_count + d.card_count);
            prop_assert_eq!(m.suspended_count, s.suspended_count + d.suspended_count);
            prop_assert_eq!(merged.live, s.card_count as usize);

            store.unmerge_deck(&merged).unwrap();
            prop_assert_eq!(dump(&store), before);
        }
    }
}
