//! Export and import after Merge Into… (ADR 0066).

use super::*;

/// The sample, plus a live "Chemistry" deck with one card, and "Maths"
/// (live, with a deleted card and reviews) merged into it. Returns the env
/// and the merge.
fn merged_env() -> (Env, crate::storage::Merged) {
    let (mut env, maths, _) = sample_env();
    let chemistry = env.store.create_deck("Chemistry").unwrap().id;
    env.store.add_card(&chemistry, "H₂O ?", "water").unwrap();
    let merged = env.store.merge_deck(&maths, &chemistry).unwrap();
    assert!(merged.cards.len() > merged.live, "a deleted card moved too");
    (env, merged)
}

#[test]
fn an_export_after_a_merge_round_trips() {
    let (source, merged) = merged_env();
    let bytes = source.export();
    let json = json_of(&bytes);
    // The merged deck is in the file, deleted, and every card it had is
    // in the target.
    let decks = json["decks"].as_array().unwrap();
    let deck = decks.iter().find(|d| d["id"] == merged.source).unwrap();
    assert_eq!(deck["deleted_at"], json!(merged.stamp));
    for card in json["cards"].as_array().unwrap() {
        assert_ne!(card["deck_id"], json!(merged.source));
    }

    let mut target = Env::new();
    target.import(&bytes).unwrap();
    assert_eq!(target.export(), bytes);
    assert_eq!(schedule_table(&target.store), schedule_table(&source.store));
    // And importing it again changes nothing.
    let plan = target.plan(&bytes).unwrap();
    assert_eq!(plan.summary().conflicts, []);
    target.import(&bytes).unwrap();
    assert_eq!(target.export(), bytes);
}

#[test]
fn an_export_after_the_undo_equals_the_one_before_the_merge() {
    let (mut env, maths, _) = sample_env();
    let chemistry = env.store.create_deck("Chemistry").unwrap().id;
    let before = env.export();
    let merged = env.store.merge_deck(&maths, &chemistry).unwrap();
    env.store.unmerge_deck(&merged).unwrap();
    assert_eq!(env.export(), before);
}

fn deck_of(store: &Store, card: &str) -> String {
    store
        .conn
        .query_row("SELECT deck_id FROM cards WHERE id = ?1", [card], |r| {
            r.get(0)
        })
        .unwrap()
}

fn deleted_at(store: &Store, card: &str) -> Option<i64> {
    store
        .conn
        .query_row("SELECT deleted_at FROM cards WHERE id = ?1", [card], |r| {
            r.get(0)
        })
        .unwrap()
}

/// A file made before the merge has the moved cards in their old deck. An
/// import never reorganises cards, so they are already present and stay
/// in the deck merged into (ADR 0067); the merged deck, live in the file,
/// is revived as any deleted deck would be, and comes back empty.
#[test]
fn an_export_from_before_a_merge_imports_as_already_present() {
    let (mut env, maths, _) = sample_env();
    let chemistry = env.store.create_deck("Chemistry").unwrap().id;
    let old = env.export();
    let merged = env.store.merge_deck(&maths, &chemistry).unwrap();

    let plan = env.plan(&old).unwrap();
    let summary = plan.summary();
    assert_eq!(summary.conflicts, []);
    assert_eq!(summary.cards.new + summary.cards.revived, 0);
    assert_eq!(
        summary.decks,
        TableCounts {
            new: 0,
            present: 2,
            revived: 1
        }
    );
    env.import(&old).unwrap();
    for card in &merged.cards {
        assert_eq!(deck_of(&env.store, card), chemistry);
    }
    assert_eq!(env.store.card_count(&maths).unwrap(), 0);
    assert_eq!(env.store.card_count(&chemistry).unwrap(), merged.live);
    // Importing it again changes nothing.
    let after = env.export();
    env.import(&old).unwrap();
    assert_eq!(env.export(), after);
}

/// The same after Move to Deck…: the card stays where it was moved, and
/// nothing is written.
#[test]
fn an_export_from_before_a_move_imports_as_already_present() {
    let (mut env, maths, card) = sample_env();
    let chemistry = env.store.create_deck("Chemistry").unwrap().id;
    let old = env.export();
    env.store
        .move_cards(std::slice::from_ref(&card), &chemistry)
        .unwrap();
    let before = env.export();

    let plan = env.plan(&old).unwrap();
    assert_eq!(plan.summary().conflicts, []);
    assert!(plan.summary().is_noop());
    env.import(&old).unwrap();
    assert_eq!(deck_of(&env.store, &card), chemistry);
    assert_eq!(env.export(), before);
    assert_ne!(maths, chemistry);
}

/// Another deck and other content: still a conflict, and nothing is
/// written.
#[test]
fn a_card_in_another_deck_with_other_content_is_a_conflict() {
    let (mut env, _, card) = sample_env();
    let chemistry = env.store.create_deck("Chemistry").unwrap().id;
    let mut file = json_of(&env.export());
    for c in file["cards"].as_array_mut().unwrap() {
        if c["id"] == json!(card) {
            c["deck_id"] = json!(chemistry);
            c["front_md"] = json!("changed");
        }
    }
    let bytes = archive_of(&file);
    let before = env.export();
    let plan = env.plan(&bytes).unwrap();
    assert_eq!(plan.summary().conflicts, [Conflict::CardDiffers(card)]);
    assert!(matches!(env.import(&bytes), Err(Error::ImportConflicts(1))));
    assert_eq!(env.export(), before);
}

/// The file has the card in a deck the database lacks: the deck is added
/// (empty), the card stays in its own deck.
#[test]
fn a_card_whose_deck_in_the_file_is_new_stays_in_its_own() {
    let (mut env, maths, card) = sample_env();
    let mut file = json_of(&env.export());
    let new_deck = "01a19d70-f61b-7f98-984f-45f461dc97ab";
    let decks = file["decks"].as_array_mut().unwrap();
    decks.push(json!({
        "id": new_deck,
        "name": "Elsewhere",
        "created_at": T0,
        "updated_at": T0,
        "deleted_at": null
    }));
    decks.sort_by_key(|d| d["id"].as_str().unwrap().to_owned());
    for c in file["cards"].as_array_mut().unwrap() {
        if c["id"] == json!(card) {
            c["deck_id"] = json!(new_deck);
        }
    }
    let bytes = archive_of(&file);
    let plan = env.plan(&bytes).unwrap();
    assert_eq!(plan.summary().conflicts, []);
    assert_eq!(plan.summary().decks.new, 1);
    assert_eq!(plan.summary().cards.new + plan.summary().cards.revived, 0);
    env.import(&bytes).unwrap();
    assert_eq!(deck_of(&env.store, &card), maths);
    assert_eq!(env.store.card_count(new_deck).unwrap(), 0);
    let after = env.export();
    env.import(&bytes).unwrap();
    assert_eq!(env.export(), after);
}

/// The card's deck in the file was deleted here since and is revived by
/// the import: it comes back empty, the card stays in its own deck.
#[test]
fn a_card_whose_deck_in_the_file_is_revived_stays_in_its_own() {
    let (mut env, maths, _) = sample_env();
    let other = env.store.create_deck("Other").unwrap().id;
    let card = env.store.add_card(&other, "q", "a").unwrap().id;
    let old = env.export();
    env.store
        .move_cards(std::slice::from_ref(&card), &maths)
        .unwrap();
    env.store.delete_deck(&other).unwrap();

    let plan = env.plan(&old).unwrap();
    assert_eq!(plan.summary().conflicts, []);
    assert_eq!(plan.summary().decks.revived, 1);
    assert_eq!(plan.summary().cards.revived, 0);
    env.import(&old).unwrap();
    assert_eq!(deck_of(&env.store, &card), maths);
    assert_eq!(deleted_at(&env.store, &card), None);
    assert_eq!(env.store.card_count(&other).unwrap(), 0);
}

/// Deleted here, live in the file, in another deck: it is revived in its
/// own deck when that deck is live after the import (live here, or
/// revived by it), and stays deleted otherwise, as it would be a live
/// card in a deleted deck.
#[test]
fn a_deleted_card_in_another_deck_is_revived_only_into_a_live_deck() {
    // Its deck here is live: revived there.
    let (mut env, maths, _) = sample_env();
    let other = env.store.create_deck("Other").unwrap().id;
    let card = env.store.add_card(&other, "q", "a").unwrap().id;
    let old = env.export();
    env.store
        .move_cards(std::slice::from_ref(&card), &maths)
        .unwrap();
    env.store.delete_card(&card).unwrap();
    let plan = env.plan(&old).unwrap();
    assert_eq!(plan.summary().cards.revived, 1);
    env.import(&old).unwrap();
    assert_eq!(
        (deck_of(&env.store, &card), deleted_at(&env.store, &card)),
        (maths.clone(), None)
    );

    // Its deck here was deleted with it and is not in the file: it stays
    // deleted where it is.
    let (mut env, _, _) = sample_env();
    let other = env.store.create_deck("Other").unwrap().id;
    let card = env.store.add_card(&other, "q", "a").unwrap().id;
    let old = env.export();
    let later = env.store.create_deck("Later").unwrap().id;
    env.store
        .move_cards(std::slice::from_ref(&card), &later)
        .unwrap();
    env.store.delete_deck(&later).unwrap();
    let stamp = deleted_at(&env.store, &card);
    let plan = env.plan(&old).unwrap();
    assert_eq!(plan.summary().conflicts, []);
    assert_eq!(plan.summary().cards.revived, 0);
    env.import(&old).unwrap();
    assert_eq!(
        (deck_of(&env.store, &card), deleted_at(&env.store, &card)),
        (later.clone(), stamp)
    );
    let after = env.export();
    env.import(&old).unwrap();
    assert_eq!(env.export(), after);

    // Its deck here was deleted with it, and the file has that deck live:
    // the import revives the deck, and the card, which the file has, follows
    // its own row (live) back into it.
    let (mut env, _, _) = sample_env();
    let other = env.store.create_deck("Other").unwrap().id;
    let later = env.store.create_deck("Later").unwrap().id;
    let card = env.store.add_card(&other, "q", "a").unwrap().id;
    let old = env.export();
    env.store
        .move_cards(std::slice::from_ref(&card), &later)
        .unwrap();
    env.store.delete_deck(&later).unwrap();
    let plan = env.plan(&old).unwrap();
    assert_eq!(plan.summary().conflicts, []);
    assert_eq!(
        (plan.summary().decks.revived, plan.summary().cards.revived),
        (1, 1)
    );
    env.import(&old).unwrap();
    assert_eq!(
        (deck_of(&env.store, &card), deleted_at(&env.store, &card)),
        (later, None)
    );
}

/// `bytes` with card `id`'s record changed by `f`.
fn with_card(bytes: &[u8], id: &str, f: impl Fn(&mut Value)) -> Vec<u8> {
    let mut file = json_of(bytes);
    for c in file["cards"].as_array_mut().unwrap() {
        if c["id"] == json!(id) {
            f(c);
        }
    }
    archive_of(&file)
}

/// Two decks, "Other" with one card, and that card moved to the sample's
/// deck since an export (which has it in "Other"). Returns the env, the
/// export, the sample's deck, "Other" and the card.
fn moved_since_export() -> (Env, Vec<u8>, String, String, String) {
    let (mut env, maths, _) = sample_env();
    let other = env.store.create_deck("Other").unwrap().id;
    let card = env.store.add_card(&other, "q", "a").unwrap().id;
    let old = env.export();
    env.store
        .move_cards(std::slice::from_ref(&card), &maths)
        .unwrap();
    (env, old, maths, other, card)
}

/// Moved here, deleted in the file: an import never deletes, so it stays
/// live where it is.
#[test]
fn a_moved_card_deleted_in_the_file_stays_live_here() {
    let (mut env, old, maths, _, card) = moved_since_export();
    let file = with_card(&old, &card, |c| c["deleted_at"] = json!(T0 + 5));
    let plan = env.plan(&file).unwrap();
    assert_eq!(plan.summary().conflicts, []);
    assert_eq!(plan.summary().cards_kept_in_their_deck, 1);
    assert!(plan.summary().is_noop());
    env.import(&file).unwrap();
    assert_eq!(
        (deck_of(&env.store, &card), deleted_at(&env.store, &card)),
        (maths, None)
    );
}

/// Moved here, then deleted on both sides: already present, nothing to
/// tell (the user sees neither).
#[test]
fn a_moved_card_deleted_on_both_sides_is_present_and_not_counted() {
    let (mut env, old, maths, _, card) = moved_since_export();
    env.store.delete_card(&card).unwrap();
    let stamp = deleted_at(&env.store, &card);
    let file = with_card(&old, &card, |c| c["deleted_at"] = json!(T0 + 5));
    let plan = env.plan(&file).unwrap();
    assert_eq!(plan.summary().conflicts, []);
    assert_eq!(plan.summary().cards_kept_in_their_deck, 0);
    assert!(plan.summary().is_noop());
    env.import(&file).unwrap();
    assert_eq!(
        (deck_of(&env.store, &card), deleted_at(&env.store, &card)),
        (maths, stamp)
    );
}

/// Moved and suspended here: both are the database's own (ADR 0035, 0067).
#[test]
fn a_moved_card_keeps_its_deck_and_its_suspension() {
    let (mut env, old, maths, _, card) = moved_since_export();
    env.store
        .set_suspended(std::slice::from_ref(&card), true)
        .unwrap();
    let plan = env.plan(&old).unwrap();
    assert_eq!(plan.summary().conflicts, []);
    assert_eq!(plan.summary().cards_kept_in_their_deck, 1);
    env.import(&old).unwrap();
    assert_eq!(deck_of(&env.store, &card), maths);
    assert!(env.store.card(&card).unwrap().suspended);
}

/// A deck revived by the import brings back the cards deleted with it,
/// but one the file has follows its own row: deleted there (in another
/// deck), it stays deleted here, in the revived deck.
#[test]
fn a_batch_card_the_file_has_deleted_in_another_deck_stays_deleted() {
    let (mut env, old, maths, _, card) = moved_since_export();
    let kept = env.store.add_card(&maths, "kept", "a").unwrap().id;
    // The sample's deck, now holding the moved card, is deleted with both.
    env.store.delete_deck(&maths).unwrap();
    let stamp = deleted_at(&env.store, &card);
    assert!(stamp.is_some());
    let file = with_card(&old, &card, |c| c["deleted_at"] = json!(T0 + 5));
    let plan = env.plan(&file).unwrap();
    assert_eq!(plan.summary().conflicts, []);
    assert_eq!(plan.summary().decks.revived, 1);
    assert_eq!(plan.summary().cards_kept_in_their_deck, 0);
    env.import(&file).unwrap();
    assert_eq!(
        (deck_of(&env.store, &card), deleted_at(&env.store, &card)),
        (maths.clone(), stamp)
    );
    // The batch card the file lacks comes back with the deck.
    assert_eq!(deleted_at(&env.store, &kept), None);
    let after = env.export();
    env.import(&file).unwrap();
    assert_eq!(env.export(), after);
}
